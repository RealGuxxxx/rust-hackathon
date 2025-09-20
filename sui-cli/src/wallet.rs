use aes_gcm::{aead::{Aead, KeyInit, OsRng}, Aes256Gcm, Nonce};
use anyhow::{anyhow, Context, Result};
use bech32::{self, FromBase32};
use blake2b_simd::Params;
use ed25519_dalek::{PublicKey, SecretKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fs::{self, File, OpenOptions};
use std::path::PathBuf;
use std::convert::TryInto;
use colored::*;

const KEY_ITERATIONS: u32 = 100_000;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Wallet {
    pub name: String,
    #[serde(default)]
    pub public_key: Option<String>, // Sui address (0x...)
    pub encrypted_private_key: String, // hex encoded
    pub salt: String,               // hex encoded
    pub nonce: String,              // hex encoded
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct WalletStore {
    pub(crate) wallets: Vec<Wallet>,
    #[serde(skip)]
    path: PathBuf,
}

impl WalletStore {
    pub fn new(path: PathBuf) -> Result<Self> {
        let mut store = if path.exists() {
            let file = File::open(&path).context("Failed to open wallet file")?;
            serde_json::from_reader(file).context("Failed to deserialize wallet file")?
        } else {
            WalletStore::default()
        };
        store.path = path;
        Ok(store)
    }

    pub fn get_wallet_path() -> Result<PathBuf> {
        let data_dir = dirs::data_dir().ok_or_else(|| anyhow!("Could not find data directory"))?;
        let app_dir = data_dir.join("sui-cli");
        fs::create_dir_all(&app_dir).context("Failed to create app directory")?;
        Ok(app_dir.join("wallets.json"))
    }

    fn save(&self) -> Result<()> {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.path)
            .context("Failed to open wallet file for writing")?;
        serde_json::to_writer_pretty(file, self).context("Failed to serialize and write wallet data")?;
        Ok(())
    }

    pub fn add_wallet(&mut self, name: &str, private_key_str: &str, password: &str) -> Result<()> {
        if self.wallets.iter().any(|w| w.name == name) {
            return Err(anyhow!("Wallet with name '{}' already exists", name));
        }

        // Derive and store public key
        let public_key = derive_sui_address(private_key_str)?;

        // Encrypt and store private key
        let mut salt = [0u8; 16];
        OsRng.fill_bytes(&mut salt);

        let key = derive_key(password, &salt);
        let cipher = Aes256Gcm::new((&key).into());

        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let encrypted_private_key = cipher
            .encrypt(nonce, private_key_str.as_bytes())
            .map_err(|e| anyhow!("Encryption failed: {}", e))?;

        let wallet = Wallet {
            name: name.to_string(),
            public_key: Some(public_key),
            encrypted_private_key: hex::encode(encrypted_private_key),
            salt: hex::encode(salt),
            nonce: hex::encode(nonce_bytes),
        };

        self.wallets.push(wallet);
        self.save()
    }

    pub fn list_wallets(&self) {
        if self.wallets.is_empty() {
            println!("\n{} {}", "⚠️".yellow(), "No wallets found.".bold());
            return;
        }
        println!("\n{} {}", "✨".cyan(), "Available wallets:".bold());
        for (i, wallet) in self.wallets.iter().enumerate() {
            let pk_str = wallet.public_key.as_deref().unwrap_or("[address not derived]");
            println!(
                "  {}. {} ({})",
                (i + 1).to_string().green(),
                wallet.name.bold(),
                pk_str.dimmed()
            );
        }
        println!(); // Add a newline for spacing
    }

    // 新增：返回钱包列表的引用
    pub fn get_wallets(&self) -> &Vec<Wallet> {
        &self.wallets
    }

    // 新增：解密私钥
    pub fn decrypt_private_key(&self, name: &str, password: &str) -> Result<String> {
        let wallet = self
            .wallets
            .iter()
            .find(|w| w.name == name)
            .ok_or_else(|| anyhow!("Wallet '{}' not found", name))?;

        let salt = hex::decode(&wallet.salt).context("Failed to decode salt")?;
        let key = derive_key(password, &salt);
        let cipher = Aes256Gcm::new((&key).into());
        let nonce_bytes = hex::decode(&wallet.nonce).context("Failed to decode nonce")?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let encrypted_private_key_bytes = hex::decode(&wallet.encrypted_private_key).context("Failed to decode private key")?;

        let decrypted_body = cipher
            .decrypt(nonce, encrypted_private_key_bytes.as_ref())
            .map_err(|_| anyhow!("Invalid password"))?;

        String::from_utf8(decrypted_body).context("Failed to convert decrypted bytes to string")
    }

    pub fn remove_wallet(&mut self, name: &str, password: &str) -> Result<()> {
        // 使用新的解密函数来验证密码
        self.decrypt_private_key(name, password)?;

        let wallet_index = self
            .wallets
            .iter()
            .position(|w| w.name == name)
            .ok_or_else(|| anyhow!("Wallet '{}' not found", name))?;

        self.wallets.remove(wallet_index);
        self.save()
    }
}

// Derive a 256-bit key from a password and salt using PBKDF2-HMAC-SHA256.
fn derive_key(password: &str, salt: &[u8]) -> [u8; 32] {
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, KEY_ITERATIONS, &mut key);
    key
}

fn derive_sui_address(private_key_str: &str) -> Result<String> {
    let pk_bytes = if private_key_str.starts_with("suiprivkey") {
        let (_, data, _) = bech32::decode(private_key_str)
            .map_err(|e| anyhow!("Invalid bech32 private key: {}", e))?;
        let decoded = Vec::<u8>::from_base32(&data).map_err(|e| anyhow!("Failed to convert from base32: {}", e))?;
        if decoded.len() != 33 || decoded[0] != 0x00 {
            return Err(anyhow!("Invalid sui private key format"));
        }
        decoded[1..].to_vec() // Return the last 32 bytes
    } else {
        hex::decode(private_key_str.trim_start_matches("0x"))
            .context("Invalid private key hex")?
    };

    let pk_bytes_array: [u8; 32] = pk_bytes.as_slice().try_into().map_err(|_| anyhow!("Invalid private key length"))?;
    let secret_key = SecretKey::from_bytes(&pk_bytes_array)
        .map_err(|e| anyhow!("Invalid private key: {}", e))?;
    let public_key: PublicKey = (&secret_key).into();

    const SUI_ADDRESS_LENGTH: usize = 32;
    const ED25519_FLAG: u8 = 0x00;

    let mut hasher = Params::new()
        .hash_length(SUI_ADDRESS_LENGTH)
        .to_state();

    hasher.update(&[ED25519_FLAG]);
    hasher.update(public_key.as_bytes());

    let g_hash = hasher.finalize();
    let address = format!("0x{}", hex::encode(g_hash.as_bytes()));
    Ok(address)
}
