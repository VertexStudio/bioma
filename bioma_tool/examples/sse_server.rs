use bioma_tool::prompts::PromptGetHandler;
use bioma_tool::resources::ResourceReadHandler;
use bioma_tool::schema::ServerCapabilities;
use bioma_tool::server::{start_with_transport, ModelContextProtocolServer};
use bioma_tool::tools::ToolCallHandler;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

struct ExampleServer {
    resources: Vec<Box<dyn ResourceReadHandler>>,
    prompts: Vec<Box<dyn PromptGetHandler>>,
    tools: Vec<Box<dyn ToolCallHandler>>,
}

impl ModelContextProtocolServer for ExampleServer {
    fn new() -> Self {
        ExampleServer { resources: Vec::new(), prompts: Vec::new(), tools: Vec::new() }
    }

    fn get_capabilities(&self) -> ServerCapabilities {
        ServerCapabilities {
            experimental: None,
            logging: None,
            prompts: Some(Default::default()),
            resources: Some(Default::default()),
            tools: Some(Default::default()),
        }
    }

    fn get_resources(&self) -> &Vec<Box<dyn ResourceReadHandler>> {
        &self.resources
    }

    fn get_prompts(&self) -> &Vec<Box<dyn PromptGetHandler>> {
        &self.prompts
    }

    fn get_tools(&self) -> &Vec<Box<dyn ToolCallHandler>> {
        &self.tools
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize the logger
    let subscriber = FmtSubscriber::builder().with_max_level(Level::DEBUG).finish();
    tracing::subscriber::set_global_default(subscriber).expect("Failed to set global default subscriber");

    // Address to bind to
    let bind_address = Some("0.0.0.0:12345".to_string());

    info!("Starting SSE server on {}", bind_address.as_ref().unwrap());

    // Start the server with SSE transport
    start_with_transport::<ExampleServer>("example-server", "sse", bind_address).await?;

    // Keep the main thread running indefinitely
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }

    #[allow(unreachable_code)]
    Ok(())
}
