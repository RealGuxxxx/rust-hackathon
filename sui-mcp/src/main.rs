#![allow(dead_code)]
use anyhow::anyhow;
use rmcp::{
    handler::server::{
        router::{prompt::PromptRouter, tool::ToolRouter},
        wrapper::Parameters,
    },
    model::*,
    prompt, prompt_handler, prompt_router,
    service::RequestContext,
    tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::env;
use std::sync::Arc;

// Import the new sui module
mod sui;
use sui::client::SuiService;

// ===========================
// ✅ 1. 定义参数和主服务结构体
// ===========================

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct TransferSuiParams {
    /// The recipient's Sui address.
    to_address: String,
    /// The amount to transfer. Can be a float (e.g., 0.5) for SUI, or a large integer for MIST.
    amount: serde_json::Value,
    /// If true, simulates the transaction and returns a summary without executing. If false, executes the transaction.
    dry_run: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct GetAssetsParams {
    /// The SUI address to check for assets. If not provided, it will use the server's own address.
    address: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct GetValueParams {
    /// The SUI address to calculate the total value for. If not provided, it will use the server's own address.
    address: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct OpenProjectParams {
    /// The name of the project to open in Suiscan.
    project_name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct SwapParams {
    /// The amount of SUI (in MIST) to swap.
    amount: u64,
}

#[derive(Clone)]
pub struct ToolService {
    sui_service: Arc<SuiService>,
    blockberry_api_key: String,
    tool_router: ToolRouter<ToolService>,
    prompt_router: PromptRouter<ToolService>,
}

// ===========================
// ✅ 2. 实现新的服务和工具
// ===========================

#[tool_router]
impl ToolService {
    pub async fn new(
        private_key_hex: String,
        blockberry_api_key: String,
    ) -> Result<Self, anyhow::Error> {
        let sui_service = SuiService::new(private_key_hex).await?;

        Ok(Self {
            sui_service: Arc::new(sui_service),
            blockberry_api_key,
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
        })
    }

    #[tool(
        description = "Prepares and optionally executes a SUI token transfer. Use dry_run=true to get a simulation summary first. After user confirmation, call again with dry_run=false to execute."
    )]
    pub async fn transfer_sui(
        &self,
        Parameters(params): Parameters<TransferSuiParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = sui::transfer::execute_transfer(
            self.sui_service.client.clone(),
            self.sui_service.keypair.clone(),
            self.sui_service.address,
            &params.to_address,
            &params.amount,
            params.dry_run,
        )
        .await;

        match result {
            Ok(summary) => Ok(CallToolResult::success(vec![Content::text(summary)])),
            Err(e) => Err(McpError::internal_error(e.to_string(), None)),
        }
    }

    #[tool(description = "Get all coins and NFTs for a given SUI address.")]
    pub async fn get_assets(
        &self,
        Parameters(params): Parameters<GetAssetsParams>,
    ) -> Result<CallToolResult, McpError> {
        let address_to_check = params
            .address
            .as_ref()
            .map_or(self.sui_service.address.to_string(), |addr| addr.clone());

        let result = sui::assets::get_all_assets(&address_to_check, &self.blockberry_api_key).await;

        match result {
            Ok(assets) => Ok(CallToolResult::success(vec![Content::text(assets)])),
            Err(e) => Err(McpError::internal_error(e.to_string(), None)),
        }
    }

    #[tool(description = "Calculate the total value of all coins in USD for a given SUI address.")]
    pub async fn get_total_value(
        &self,
        Parameters(params): Parameters<GetValueParams>,
    ) -> Result<CallToolResult, McpError> {
        let address_to_check = params
            .address
            .as_ref()
            .map_or(self.sui_service.address.to_string(), |addr| addr.clone());

        let result = sui::assets::calculate_wallet_value(&address_to_check, &self.blockberry_api_key).await;

        match result {
            Ok(value) => {
                let result_string = format!("Total wallet value for {}: ${:.2} USD", address_to_check, value);
                Ok(CallToolResult::success(vec![Content::text(result_string)]))
            }
            Err(e) => Err(McpError::internal_error(e.to_string(), None)),
        }
    }

    #[tool(description = "Get the top 5 hottest DeFi projects by Total Value Locked (TVL).")]
    pub async fn get_top_defi_projects(&self) -> Result<CallToolResult, McpError> {
        let result = sui::assets::get_top_defi_projects(&self.blockberry_api_key).await;
        match result {
            Ok(projects_list) => Ok(CallToolResult::success(vec![Content::text(projects_list)])),
            Err(e) => Err(McpError::internal_error(e.to_string(), None)),
        }
    }

    #[tool(description = "Opens the Suiscan page for a given project in the default web browser.")]
    pub async fn open_project_in_browser(
        &self,
        Parameters(params): Parameters<OpenProjectParams>,
    ) -> Result<CallToolResult, McpError> {
        let url = format!("https://suiscan.xyz/mainnet/directory/{}", params.project_name);
        match opener::open(&url) {
            Ok(_) => Ok(CallToolResult::success(vec![Content::text(format!("Successfully opened {} in your browser.", url))])),
            Err(e) => Err(McpError::internal_error(format!("Failed to open browser: {}", e), None)),
        }
    }

}

// ===========================
// ✅ 3. 更新 Handler 和 main 函数
// ===========================

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct ExamplePromptArgs {
    pub message: String,
}

#[prompt_router]
impl ToolService {
    #[prompt(name = "example_prompt")]
    async fn example_prompt(
        &self,
        Parameters(args): Parameters<ExamplePromptArgs>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        let prompt = format!(
            "This is an example prompt with your message here: '{}'",
            args.message
        );
        Ok(vec![PromptMessage {
            role: PromptMessageRole::User,
            content: PromptMessageContent::text(prompt),
        }])
    }
}

#[tool_handler]
#[prompt_handler]
impl ServerHandler for ToolService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder()
                .enable_prompts()
                .enable_tools()
                .build(),
            server_info: Implementation::from_build_env(),
            instructions: Some("This server provides Sui network tools. You can transfer SUI with `transfer_sui` and check assets with `get_assets`.".to_string()),
        }
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParam>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: vec![],
            next_cursor: None,
        })
    }

    async fn read_resource(
        &self,
        ReadResourceRequestParam { uri }: ReadResourceRequestParam,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        Err(McpError::resource_not_found(
            "resource_not_found",
            Some(json!({ "uri": uri })),
        ))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParam>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult {
            next_cursor: None,
            resource_templates: Vec::new(),
        })
    }

    async fn initialize(
        &self,
        _request: InitializeRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        if let Some(http_request_part) = context.extensions.get::<axum::http::request::Parts>() {
            let initialize_headers = &http_request_part.headers;
            let initialize_uri = &http_request_part.uri;
            tracing::info!(?initialize_headers, %initialize_uri, "initialize from http server");
        }
        Ok(self.get_info())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure file logging to the system's temporary directory
    let temp_dir = std::env::temp_dir();
    let file_appender = tracing_appender::rolling::never(temp_dir, "sui-mcp-debug.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let private_key = env::var("SUI_PRIVATE_KEY")
        .map_err(|_| anyhow!("SUI_PRIVATE_KEY environment variable not set"))?;

    // It's good practice to remove the key from the environment after reading it.
    env::remove_var("SUI_PRIVATE_KEY");
    
    let blockberry_api_key = env::var("BLOCKBERRY_API_KEY")
        .unwrap_or_else(|_| "CboBilt0ncYQi24Zxyzz17n7UpwcXC".to_string());

    let service = ToolService::new(private_key, blockberry_api_key).await?;

    let served = service
        .serve((tokio::io::stdin(), tokio::io::stdout()))
        .await
        .inspect_err(|err| {
            tracing::error!("Error: {}", err);
        })?;

    served.waiting().await.unwrap();

    Ok(())
}
