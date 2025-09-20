mod app;
mod cli;
mod grpc_client;
mod input;
mod ui;
mod wallet;

use crate::cli::{Cli, Commands, WalletAction};
use crate::wallet::{Wallet, WalletStore};
use app::App;
use clap::Parser;
use grpc_client::{agent::agent_service_client::AgentServiceClient, send_chat};
use input::handle_input;
use rpassword::read_password;
use ui::{draw_ui, TuiTerminal};

use anyhow::{anyhow, Context, Result};
use colored::*;

use crossterm::{event, terminal, ExecutableCommand};
use ratatui::backend::CrosstermBackend;
use std::io::{self, stdin, Write};
use std::path::Path;
use std::process::{Child, Command};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;

// RAII guard to ensure the child process is killed on exit.
struct ChildProcessGuard(Child);

impl Drop for ChildProcessGuard {
    fn drop(&mut self) {
        println!("\nShutting down agent process...");
        if let Err(e) = self.0.kill() {
            eprintln!("Failed to kill agent process: {}", e);
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let wallet_path = WalletStore::get_wallet_path()?;
    let mut wallet_store = WalletStore::new(wallet_path)?;

    match cli.command {
        Commands::Cli => {
            // Pass the wallet_store to the cli runner
            if let Err(e) = run_cli(&wallet_store).await {
                // First, exit the alternate screen and disable raw mode to restore the terminal
                let _ = terminal::disable_raw_mode();
                let _ = io::stdout().execute(terminal::LeaveAlternateScreen);

                // Now, print the error to the standard terminal
                eprintln!("\n--- APPLICATION ERROR ---\n");
                eprintln!("{:#}", e);
                eprintln!("\n-------------------------\n");
                eprintln!("The application will exit in 15 seconds.");

                // Wait for 15 seconds
                sleep(Duration::from_secs(15)).await;

                std::process::exit(1);
            }
        }
        Commands::Wallet(wallet_args) => match wallet_args.action {
            WalletAction::Import(import_args) => {
                let private_key_str =
                    read_password_from_prompt("Enter private key (hex or bech32): ")?;
                let password =
                    read_password_from_prompt("Enter a password to encrypt the private key: ")?;
                let confirm_password = read_password_from_prompt("Confirm password: ")?;

                if password != confirm_password {
                    eprintln!("Passwords do not match.");
                    return Ok(());
                }

                match wallet_store.add_wallet(
                    &import_args.account_name,
                    &private_key_str,
                    &password,
                ) {
                    Ok(_) => {
                        println!(
                            "Wallet '{}' imported successfully.",
                            import_args.account_name
                        )
                    }
                    Err(e) => eprintln!("Error: {}", e),
                }
            }
            WalletAction::List => {
                wallet_store.list_wallets();
            }
            WalletAction::Remove(remove_args) => {
                let password = read_password_from_prompt("Enter password to confirm removal: ")?;
                match wallet_store.remove_wallet(&remove_args.account_name, &password) {
                    Ok(_) => println!(
                        "Wallet '{}' removed successfully.",
                        remove_args.account_name
                    ),
                    Err(e) => eprintln!("Error: {}", e),
                }
            }
        },
    }

    Ok(())
}

fn read_password_from_prompt(prompt: &str) -> Result<String> {
    print!("{}", prompt);
    io::stdout().flush()?;
    read_password().map_err(Into::into)
}

async fn run_cli(wallet_store: &WalletStore) -> Result<()> {
    // --- 1. Wallet Selection and Decryption ---
    let wallets = wallet_store.get_wallets();
    if wallets.is_empty() {
        return Err(anyhow!(
            "No wallets found. Please import a wallet first using the 'wallet import' command."
        ));
    }

    println!("\n{}", "✨ Please select a wallet to use:".bold().cyan());
    for (i, wallet) in wallets.iter().enumerate() {
        println!(
            "  {}. {} ({})",
            (i + 1).to_string().green(),
            wallet.name.bold(),
            wallet.public_key.as_deref().unwrap_or("N/A").dimmed()
        );
    }

    let selected_wallet: &Wallet;
    loop {
        print!("{}", "👉 Enter number: ".bold().yellow());
        io::stdout().flush()?;
        let mut input = String::new();
        stdin().read_line(&mut input)?;
        match input.trim().parse::<usize>() {
            Ok(n) if n > 0 && n <= wallets.len() => {
                selected_wallet = &wallets[n - 1];
                break;
            }
            _ => println!("Invalid selection. Please try again."),
        }
    }
    println!(); // Add a newline for spacing

    let password =
        read_password_from_prompt(&format!("Enter password for '{}': ", selected_wallet.name))?;
    let private_key = wallet_store.decrypt_private_key(&selected_wallet.name, &password)?;
    println!(
        "\n{} {}",
        "✅".green(),
        "Wallet unlocked. Starting agent...".bold()
    );

    // --- 2. Launch Agent Process ---
    // Construct the path to the virtual environment's python executable
    let venv_python = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("agent-py")
        .join(".venv")
        .join("bin")
        .join("python");

    // Debug: Print the path to be used

    let agent_command = Command::new(&venv_python)
        .arg("../agent-py/main.py")
        .env("SUI_PRIVATE_KEY", private_key)
        .env("PYTHONUNBUFFERED", "1")
        .stdout(std::process::Stdio::null()) // Redirect stdout to /dev/null to keep TUI clean
        // All fixed! Redirect stderr to null again for a clean UI.
        .stderr(std::process::Stdio::null())
        .spawn();

    let agent_process = match agent_command {
        Ok(process) => process,
        Err(e) => {
            // Explicitly print the error and the path that was used
            eprintln!("\n--- AGENT SPAWN FAILED ---");
            eprintln!("Error: {}", e);
            eprintln!("Failed to execute Python from path: {:?}", venv_python);
            eprintln!("Please check if the path is correct and the file is executable.");
            eprintln!("--------------------------");
            // Return a detailed error to the main handler
            return Err(anyhow!(
                "Failed to spawn agent-py process: {}. Path: {:?}",
                e,
                venv_python
            ));
        }
    };

    let _guard = ChildProcessGuard(agent_process);

    // --- 3. Connect to Agent gRPC Server with Retry ---
    println!(
        "\u{23f0}{} {}",
        "⏳".yellow(),
        "Waiting for agent to start...".bold()
    );
    let client = {
        const MAX_RETRIES: u32 = 90; // 90 retries * 2s delay = 3 minutes total timeout.
        const RETRY_DELAY: Duration = Duration::from_secs(1); // Use 1s delay for a smoother animation
        let mut client = None;

        let spinner_chars = ['~', '-', '≈', '-'];
        print!(
            "{} {}",
            "🚀".blue(),
            "Agent is initializing, this may take a few minutes on the first run... ".bold()
        );
        io::stdout().flush().unwrap();

        for i in 0..MAX_RETRIES {
            if let Ok(c) = AgentServiceClient::connect("http://[::1]:50051").await {
                client = Some(c);
                println!("{} {}", "🎉".green(), "Ready!".bold());
                break;
            }

            if i < MAX_RETRIES - 1 {
                let spinner_char = spinner_chars[i as usize % spinner_chars.len()];
                print!("{} ", spinner_char);
                io::stdout().flush().unwrap();
                sleep(RETRY_DELAY).await;
                print!("\u{8}\u{8}"); // Backspace twice to erase char and space
            }
        }

        if client.is_none() {
            println!("{} {}", "❌".red(), "Failed!".bold());
        }

        client.context("Failed to connect to the agent gRPC server after the timeout. The agent may have failed to start.")?
    };
    println!(
        "\n{} {}",
        "✨".green(),
        "Agent connected successfully!".bold()
    );
    sleep(Duration::from_secs(1)).await;

    // --- 4. Run TUI ---
    let (tx, rx) = mpsc::channel(100);
    let mut app = App::new(rx, tx.clone());

    let mut stdout = io::stdout();
    terminal::enable_raw_mode()?;
    stdout.execute(terminal::EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = TuiTerminal::new(backend)?;

    let mut running = true;
    while running {
        draw_ui(&mut app, &mut terminal)?;

        if event::poll(Duration::from_millis(100))? {
            let (is_running, user_msg) = handle_input(&mut app)?;
            running = is_running;

            if let Some(msg) = user_msg {
                app.push_message("You".to_string(), msg.clone());
                app.start_streaming();
                let tx_clone = app.tx.clone();
                let client_clone = client.clone();
                tokio::spawn(async move {
                    send_chat(client_clone, msg, tx_clone).await;
                });
            }
        }

        while let Ok(msg) = app.rx.try_recv() {
            if msg == "[STREAM_END]" {
                app.finalize_ai_message();
            } else {
                app.append_stream(&msg);
            }
        }
    }

    // --- 5. Cleanup ---
    terminal::disable_raw_mode()?;
    io::stdout().execute(terminal::LeaveAlternateScreen)?;

    Ok(())
}
