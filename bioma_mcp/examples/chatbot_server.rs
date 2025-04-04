use anyhow::Result;
use bioma_mcp::{
    prompts::PromptGetHandler,
    resources::ResourceReadHandler,
    schema::{ServerCapabilities, ServerCapabilitiesPromptsResourcesTools},
    server::{Context, ModelContextProtocolServer, Pagination, Server, StdioConfig, TransportConfig},
    tools::{chat, ToolCallHandler},
};
use std::sync::Arc;
use tracing::{error, info};

pub struct ChatBotServer {
    transport_config: TransportConfig,
    capabilities: ServerCapabilities,
    pagination: Pagination,
}

impl ChatBotServer {
    pub fn new(transport_config: TransportConfig) -> Self {
        let capabilities = ServerCapabilities {
            tools: Some(ServerCapabilitiesPromptsResourcesTools { list_changed: Some(false) }),
            ..Default::default()
        };

        Self { transport_config, capabilities: capabilities, pagination: Pagination { size: 20 } }
    }
}

impl ModelContextProtocolServer for ChatBotServer {
    async fn get_transport_config(&self) -> TransportConfig {
        self.transport_config.clone()
    }

    async fn get_capabilities(&self) -> ServerCapabilities {
        self.capabilities.clone()
    }

    async fn get_pagination(&self) -> Option<Pagination> {
        Some(self.pagination.clone())
    }

    async fn new_resources(&self, _context: Context) -> Vec<Arc<dyn ResourceReadHandler>> {
        vec![]
    }

    async fn new_prompts(&self, _context: Context) -> Vec<Arc<dyn PromptGetHandler>> {
        vec![]
    }

    async fn new_tools(&self, context: Context) -> Vec<Arc<dyn ToolCallHandler>> {
        info!("Registerin chat tool");
        vec![Arc::new(chat::Chat::new(context))]
    }

    async fn on_error(&self, error: anyhow::Error) -> () {
        error!("Error: {}", error);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let server = ChatBotServer::new(TransportConfig::Stdio(StdioConfig {}));

    let mcp_server = Server::new(server);

    mcp_server.start().await?;

    Ok(())
}
