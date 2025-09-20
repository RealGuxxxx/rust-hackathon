
use anyhow::anyhow;
use std::sync::Arc;
use sui_sdk::{SuiClient, SuiClientBuilder};
use sui_types::base_types::SuiAddress;
use sui_types::crypto::SuiKeyPair;

#[derive(Clone)]
pub struct SuiService {
    pub client: Arc<SuiClient>,
    pub keypair: Arc<SuiKeyPair>,
    pub address: SuiAddress,
}

impl SuiService {
    pub async fn new(private_key_hex: String) -> Result<Self, anyhow::Error> {
        let keypair = SuiKeyPair::decode(&private_key_hex)
            .map_err(|e| anyhow!("Failed to parse private key: {}", e))?;

        let address: SuiAddress = (&keypair.public()).into();

        let client = SuiClientBuilder::default()
            .build("https://fullnode.testnet.sui.io:443")
            .await?;

        Ok(Self {
            client: Arc::new(client),
            keypair: Arc::new(keypair),
            address,
        })
    }
}
