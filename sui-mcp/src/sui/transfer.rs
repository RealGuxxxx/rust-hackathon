use anyhow::{anyhow, Result};
use std::str::FromStr;
use std::sync::Arc;
use futures::StreamExt;
use sui_sdk::rpc_types::{SuiExecutionStatus, SuiTransactionBlockEffectsAPI, SuiTransactionBlockResponseOptions};
use sui_sdk::SuiClient;
use sui_types::base_types::{SuiAddress, ObjectRef};
use sui_types::crypto::{Signer, SuiKeyPair, Signature};
use sui_types::programmable_transaction_builder::ProgrammableTransactionBuilder;
use sui_types::quorum_driver_types::ExecuteTransactionRequestType;
use sui_types::transaction::{Transaction, TransactionData};


pub async fn execute_transfer(
    sui_client: Arc<SuiClient>,
    keypair: Arc<SuiKeyPair>,
    from_address: SuiAddress,
    to_address_str: &str,
    amount_json: &serde_json::Value,
    dry_run: bool,
) -> Result<String> {
    let amount_mist = match amount_json {
        serde_json::Value::Number(n) => {
            if n.is_f64() && n.as_f64().unwrap_or(0.0).fract() != 0.0 {
                (n.as_f64().unwrap() * 1_000_000_000.0) as u64
            } else {
                n.as_u64().unwrap_or_else(|| n.as_f64().unwrap_or(0.0) as u64)
            }
        }
        serde_json::Value::String(s) => {
            if s.contains('.') {
                match s.parse::<f64>() {
                    Ok(f) => (f * 1_000_000_000.0) as u64,
                    Err(_) => return Err(anyhow!("Invalid 'amount' string format for SUI.")),
                }
            } else if let Ok(i) = s.parse::<u64>() {
                i
            } else {
                return Err(anyhow!("Invalid 'amount' string format. Must be a whole number for MIST."));
            }
        }
        _ => return Err(anyhow!("The 'amount' parameter must be a number or a string.")),
    };

    let to_address = SuiAddress::from_str(to_address_str)
        .map_err(|e| anyhow!("Invalid recipient address: {}", e))?;

    let gas_budget = 50_000_000;
    let required_balance = amount_mist + gas_budget;

    let coins: Vec<_> = sui_client
        .coin_read_api()
        .get_coins_stream(from_address, Some("0x2::sui::SUI".to_string()))
        .collect()
        .await;

    if coins.is_empty() {
        return Err(anyhow!("No SUI coins found for the address."));
    }

    let mut selected_coins: Vec<ObjectRef> = Vec::new();
    let mut total_balance = 0;
    for coin in coins {
        total_balance += coin.balance;
        selected_coins.push(coin.object_ref());
        if total_balance >= required_balance {
            break;
        }
    }

    if total_balance < required_balance {
        return Err(anyhow!(
            "Insufficient SUI balance. Required: {}, Available: {}",
            required_balance, total_balance
        ));
    }

    let pt = {
        let mut builder = ProgrammableTransactionBuilder::new();
        builder.transfer_sui(to_address, Some(amount_mist));
        builder.finish()
    };

    let gas_price = sui_client
        .read_api()
        .get_reference_gas_price()
        .await?;

    let tx_data = TransactionData::new_programmable(
        from_address,
        selected_coins,
        pt,
        gas_budget,
        gas_price,
    );

    if dry_run {
        let simulation_result = sui_client
            .read_api()
            .dry_run_transaction_block(tx_data.clone())
            .await
            .map_err(|e| anyhow!("Transaction simulation failed: {}", e))?;

        if simulation_result.effects.status() != &SuiExecutionStatus::Success {
            return Err(anyhow!(
                "Simulation was not successful: {:?}",
                simulation_result.effects.status()
            ));
        }

        let amount_sui = amount_mist as f64 / 1_000_000_000.0;
        let gas_summary = simulation_result.effects.gas_cost_summary();
        let total_gas_cost =
            gas_summary.computation_cost + gas_summary.storage_cost - gas_summary.storage_rebate;

        let summary = format!(
            "Transaction Simulation Summary:\n- From: {}\n- To: {}\n- Amount: {} SUI\n- Estimated Gas Cost: {} MIST\n\nTo execute, please confirm.",
            from_address,
            to_address,
            amount_sui,
            total_gas_cost
        );
        Ok(summary)
    } else {
        let transaction = Transaction::from_data_and_signer(
            tx_data,
            vec![&*keypair as &dyn Signer<Signature>],
        );

        let transaction_response = sui_client
            .quorum_driver_api()
            .execute_transaction_block(
                transaction,
                SuiTransactionBlockResponseOptions::full_content(),
                Some(ExecuteTransactionRequestType::WaitForLocalExecution),
            )
            .await
            .map_err(|e| anyhow!("Transaction execution failed: {}", e))?;

        let result_summary = format!("Transaction executed successfully! Digest: {}", transaction_response.digest);
        Ok(result_summary)
    }
}
