use anyhow::{Context, Result};
use bioma_mcp::{
    prompts::{self, PromptGetHandler},
    resources::{self, ResourceReadHandler},
    schema::{
        ServerCapabilities, ServerCapabilitiesPrompts, ServerCapabilitiesPromptsResources,
        ServerCapabilitiesPromptsResourcesTools,
    },
    server::{ModelContextProtocolServer, Server, SseConfig, StdioConfig, TransportConfig, WsConfig},
    tools::{self, ToolCallHandler},
};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, Level};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::format::FmtSpan;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, short, default_value = "mcp_server.log")]
    log_file: PathBuf,

    #[arg(long, short, default_value = ".")]
    base_dir: PathBuf,

    #[command(subcommand)]
    transport: Transport,
}

#[derive(Subcommand)]
enum Transport {
    Stdio,

    Sse {
        #[arg(long, short, default_value = "127.0.0.1:8090")]
        endpoint: String,
    },

    Ws {
        #[arg(long, short, default_value = "127.0.0.1:9090")]
        endpoint: String,
    },
}

struct ExampleMcpServer {
    transport_config: TransportConfig,
    capabilities: ServerCapabilities,
    resources: Vec<Box<dyn ResourceReadHandler>>,
    prompts: Vec<Box<dyn PromptGetHandler>>,
}

impl ModelContextProtocolServer for ExampleMcpServer {
    fn get_transport_config(&self) -> &TransportConfig {
        &self.transport_config
    }

    fn get_capabilities(&self) -> &ServerCapabilities {
        &self.capabilities
    }

    fn get_resources(&self) -> &Vec<Box<dyn ResourceReadHandler>> {
        &self.resources
    }

    fn get_prompts(&self) -> &Vec<Box<dyn PromptGetHandler>> {
        &self.prompts
    }

    fn create_tools(&self) -> Vec<Arc<dyn ToolCallHandler>> {
        vec![
            Arc::new(tools::echo::Echo),
            Arc::new(tools::memory::Memory),
            Arc::new(tools::fetch::Fetch::default()),
            Arc::new(tools::random::RandomNumber),
            Arc::new(tools::workflow::Workflow::new(true, None)),
        ]
    }
}

fn setup_logging(log_path: PathBuf) -> Result<()> {
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).context("Failed to create log directory")?;
    }

    let file_appender = RollingFileAppender::new(
        Rotation::NEVER,
        log_path.parent().unwrap_or(&PathBuf::from(".")),
        log_path.file_name().unwrap_or_default(),
    );

    tracing_subscriber::fmt()
        .with_timer(tracing_subscriber::fmt::time::UtcTime::rfc_3339())
        .with_level(true)
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .with_ansi(false)
        .with_span_events(FmtSpan::CLOSE)
        .with_writer(file_appender)
        .with_max_level(Level::DEBUG)
        .init();

    info!("Logging system initialized");

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    setup_logging(args.log_file)?;

    let transport_config = match &args.transport {
        Transport::Stdio => TransportConfig::Stdio(StdioConfig {}),
        Transport::Sse { endpoint } => TransportConfig::Sse(SseConfig::builder().endpoint(endpoint.clone()).build()),
        Transport::Ws { endpoint } => TransportConfig::Ws(WsConfig::builder().endpoint(endpoint.clone()).build()),
    };

    let capabilities = ServerCapabilities {
        tools: Some(ServerCapabilitiesPromptsResourcesTools { list_changed: Some(false) }),
        resources: Some(ServerCapabilitiesPromptsResources { list_changed: Some(true), subscribe: Some(true) }),
        prompts: Some(ServerCapabilitiesPrompts { list_changed: Some(false) }),
        ..Default::default()
    };

    let resources: Vec<Box<dyn ResourceReadHandler>> = vec![
        Box::new(resources::readme::Readme),
        Box::new(resources::filesystem::FileSystem::new(args.base_dir.clone())),
    ];

    let prompts: Vec<Box<dyn PromptGetHandler>> = vec![Box::new(prompts::greet::Greet)];

    let server = ExampleMcpServer { transport_config, capabilities, resources, prompts };

    let mcp_server = Server::new(server);

    mcp_server.start().await
}
