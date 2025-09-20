use anyhow::{anyhow, Result};
use serde::Deserialize;
use tracing::{error, info};

// Generic wrapper for the paginated API response
#[derive(Deserialize, Debug, PartialEq)]
pub struct ApiResponse<T> {
    pub content: Vec<T>,
}


#[derive(Deserialize, Debug, PartialEq)]
pub struct Nft {
    #[serde(rename = "objectName")]
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub item_type: String,
}


#[derive(Deserialize, Debug, PartialEq)]
pub struct Coin {
    #[serde(rename = "totalBalance")]
    pub balance: u64,
    #[serde(rename = "coinType")]
    pub coin_type: String,
    #[serde(rename = "coinSymbol")]
    pub symbol: String,
    pub decimals: i32,
    #[serde(rename = "coinPrice")]
    pub price: Option<f64>,
}


async fn get_nfts(client: &reqwest::Client, api_key: &str, address: &str) -> Result<ApiResponse<Nft>> {
    let url = format!("https://api.blockberry.one/sui/v1/nfts/wallet/{}?page=0&size=100&orderBy=DESC&sortBy=AGE", address);
    info!(target: "sui_mcp_log", "Requesting NFTs from: {}", url);

    let response = client.get(&url)
        .header("x-api-key", api_key)
        .send()
        .await
        .map_err(|e| {
            error!(target: "sui_mcp_log", "Network error fetching NFTs: {}", e);
            anyhow!("Network error fetching NFTs: {}", e)
        })?;

    let status = response.status();
    info!(target: "sui_mcp_log", "Received NFT API status: {}", status);

    if status.is_success() {
        response.json::<ApiResponse<Nft>>().await.map_err(|e| {
            error!(target: "sui_mcp_log", "Failed to parse NFT JSON response: {}", e);
            anyhow!("Failed to parse NFT JSON response: {}", e)
        })
    } else {
        let error_text = response.text().await.unwrap_or_else(|_| "<failed to read error body>".to_string());
        error!(target: "sui_mcp_log", "API error fetching NFTs (status {}): {}", status, error_text);
        Err(anyhow!("API error fetching NFTs: {}", error_text))
    }
}

async fn get_coins(client: &reqwest::Client, api_key: &str, address: &str) -> Result<ApiResponse<Coin>> {
    let url = format!("https://api.blockberry.one/sui/v1/coins/wallet/{}?page=0&size=100", address);
    info!(target: "sui_mcp_log", "Requesting coins from: {}", url);

    let response = client.get(&url)
        .header("x-api-key", api_key)
        .send()
        .await
        .map_err(|e| {
            error!(target: "sui_mcp_log", "Network error fetching coins: {}", e);
            anyhow!("Network error fetching coins: {}", e)
        })?;

    let status = response.status();
    info!(target: "sui_mcp_log", "Received Coin API status: {}", status);

    if status.is_success() {
        response.json::<ApiResponse<Coin>>().await.map_err(|e| {
            error!(target: "sui_mcp_log", "Failed to parse Coin JSON response: {}", e);
            anyhow!("Failed to parse Coin JSON response: {}", e)
        })
    } else {
        let error_text = response.text().await.unwrap_or_else(|_| "<failed to read error body>".to_string());
        error!(target: "sui_mcp_log", "API error fetching coins (status {}): {}", status, error_text);
        Err(anyhow!("API error fetching coins: {}", error_text))
    }
}

pub async fn get_all_assets(address: &str, api_key: &str) -> Result<String> {
    info!(target: "sui_mcp_log", "Starting asset fetch for address: {}", address);
    let client = reqwest::Client::new();

    let nfts_future = get_nfts(&client, api_key, address);
    let coins_future = get_coins(&client, api_key, address);

    match tokio::try_join!(nfts_future, coins_future) {
        Ok((nfts_response, coins_response)) => {
            let nfts = nfts_response.content;
            let coins = coins_response.content;
            info!(target: "sui_mcp_log", "Successfully fetched {} coins and {} nfts", coins.len(), nfts.len());
            
            let mut result = String::new();
            let header = format!("Assets for address: {}\n\n", address);
            result.push_str(&header);

            result.push_str("---" );
            result.push_str(" Coins ---
");
            if coins.is_empty() {
                result.push_str("No coins found.\n");
            } else {
                for coin in coins {
                    let formatted_balance = if coin.coin_type == "0x2::sui::SUI" {
                        format!("{:.9} SUI", coin.balance as f64 / 1_000_000_000.0)
                    } else {
                        format!("{} {}", coin.balance, coin.symbol)
                    };
                    let line = format!("- {}: {}\n", coin.symbol, formatted_balance);
                    result.push_str(&line);
                }
            }

            result.push_str("\n--- NFTs ---
");
            if nfts.is_empty() {
                result.push_str("No NFTs found.\n");
            } else {
                for nft in nfts {
                    let name = nft.name.unwrap_or_else(|| "[No Name]".to_string());
                    let line = format!("- Name: {}\n  Type: {}\n", name, nft.item_type);
                    result.push_str(&line);
                }
            }

            Ok(result)
        }
        Err(e) => {
            error!(target: "sui_mcp_log", "Failed to fetch assets: {}", e);
            Err(anyhow!("Failed to fetch assets: {}", e))
        }
    }
}

pub async fn calculate_wallet_value(address: &str, api_key: &str) -> Result<f64> {
    info!(target: "sui_mcp_log", "Starting wallet value calculation for address: {}", address);
    let client = reqwest::Client::new();
    let coins_response = get_coins(&client, api_key, address).await?;
    
    let mut total_value = 0.0;

    for coin in coins_response.content {
        if let Some(price) = coin.price {
            let balance = coin.balance as f64;
            let decimals = coin.decimals;
            let adjusted_balance = balance / 10.0_f64.powi(decimals);
            total_value += adjusted_balance * price;
        }
    }

    Ok(total_value)
}

#[derive(Deserialize, Debug, PartialEq)]
pub struct DeFiProject {
    pub name: String,
    #[serde(rename = "currTvl")]
    pub tvl: f64,
}

pub async fn get_top_defi_projects(api_key: &str) -> Result<String> {
    info!(target: "sui_mcp_log", "Fetching top DeFi projects");
    let client = reqwest::Client::new();
    let url = "https://api.blockberry.one/sui/v1/widgets/defi/projects";

    let response = client.get(url).header("x-api-key", api_key).send().await.map_err(|e| {
        error!(target: "sui_mcp_log", "Network error fetching DeFi projects: {}", e);
        anyhow!("Network error fetching DeFi projects: {}", e)
    })?;

    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_else(|_| "<failed to read error body>".to_string());
        error!(target: "sui_mcp_log", "API error fetching DeFi projects: {}", error_text);
        return Err(anyhow!("API error fetching DeFi projects: {}", error_text));
    }

    let projects = response.json::<Vec<DeFiProject>>().await.map_err(|e| {
        error!(target: "sui_mcp_log", "Failed to parse DeFi projects JSON: {}", e);
        anyhow!("Failed to parse DeFi projects JSON: {}", e)
    })?;

    let mut result = "Top 5 DeFi Projects by TVL:\n".to_string();
    for (i, project) in projects.iter().take(5).enumerate() {
        let line = format!("  {}. {}: ${:.2} USD\n", i + 1, project.name, project.tvl);
        result.push_str(&line);
    }

    Ok(result)
}



#[cfg(test)]
mod tests {
    use super::*; 

    #[test]
    fn test_deserialize_coin_response() {
        let json_data = r#"{
          "size": 20,
          "totalPages": 3,
          "totalCount": 57,
          "content": [
            {
              "coinType": "0x4c981f3ff786cdb9e514da897ab8a953647dae2ace9679e8358eec1e3e8871ac::dmc::DMC",
              "coinName": "DeLorean",
              "coinDenom": "DMC",
              "decimals": 9,
              "coinSymbol": "DMC",
              "objectType": "Coin",
              "objectsCount": 1,
              "lockedBalance": null,
              "totalBalance": 2000000000,
              "coinPrice": 0.00418963,
              "imgUrl": "https://storage.googleapis.com/tokenimage.deloreanlabs.com/DMCTokenIcon.svg",
              "securityMessage": null,
              "bridged": false,
              "hasNoMetadata": false,
              "verified": true
            }
          ],
          "pageable": {
            "sort": {
              "sorted": false,
              "empty": true,
              "unsorted": true
            },
            "pageNumber": 0,
            "pageSize": 20,
            "offset": 0,
            "paged": true,
            "unpaged": false
          },
          "last": false,
          "totalElements": 57,
          "number": 0,
          "sort": {
            "sorted": false,
            "empty": true,
            "unsorted": true
          },
          "first": true,
          "numberOfElements": 20,
          "empty": false
        }"#;

        let result = serde_json::from_str::<ApiResponse<Coin>>(json_data);
        assert!(result.is_ok(), "Failed to deserialize coin response: {:?}", result.err());
        let parsed_data = result.unwrap();
        assert_eq!(parsed_data.content.len(), 1);
        let coin = &parsed_data.content[0];
        assert_eq!(coin.symbol, "DMC");
        assert_eq!(coin.balance, 2000000000);
        assert_eq!(coin.decimals, 9);
        assert_eq!(coin.price, Some(0.00418963));
    }

    #[test]
    fn test_deserialize_nft_response() {
        let json_data = r#"{
          "content": [
            {
              "id": "0x3173a8a8fdc128c6a090109dbafc57a261107658726aa2fd4cc56f7794519b13",
              "type": "0x58156e414780a5a237db71afb0d852674eff8cd98f9572104cb79afeb4ad1e9d::suinet::SUITOMAINNET",
              "objectName": "Quest 3 Rewards Live",
              "imgUrl": "https://i.imgur.com/8JYWNI7.png",
              "description": "Quest 3 5 million sui rewards distribution.",
              "amount": null,
              "latestPrice": null
            },
            {
              "id": "0xsome_other_id",
              "type": "0xsome_other_type",
              "objectName": null,
              "imgUrl": null,
              "description": null,
              "amount": null,
              "latestPrice": null
            }
          ],
          "pageable": {
            "sort": {
              "sorted": true,
              "empty": false,
              "unsorted": false
            },
            "pageNumber": 0,
            "pageSize": 1,
            "offset": 0,
            "paged": true,
            "unpaged": false
          },
          "last": false,
          "totalPages": 29,
          "totalElements": 29,
          "size": 1,
          "number": 0,
          "sort": {
            "sorted": true,
            "empty": false,
            "unsorted": false
          },
          "first": true,
          "numberOfElements": 1,
          "empty": false
        }"#;

        let result = serde_json::from_str::<ApiResponse<Nft>>(json_data);
        assert!(result.is_ok(), "Failed to deserialize nft response: {:?}", result.err());
        let parsed_data = result.unwrap();
        assert_eq!(parsed_data.content.len(), 2);
        assert_eq!(parsed_data.content[0].name, Some("Quest 3 Rewards Live".to_string()));
        assert_eq!(parsed_data.content[1].name, None);
    }
}