use anyhow::{Context, Result};
use bioma_tool::{
    prompts::{self, PromptGetHandler},
    resources::{self, ResourceReadHandler},
    schema::{
        ServerCapabilities, ServerCapabilitiesPrompts, ServerCapabilitiesPromptsResources,
        ServerCapabilitiesPromptsResourcesTools,
    },
    server::{ModelContextProtocolServer, SseConfig, StdioConfig, TransportConfig, WsConfig},
    tools::{self, ToolCallHandler},
};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::{info, Level};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::format::FmtSpan;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the log file
    #[arg(long, short, default_value = "mcp_server.log")]
    log_file: PathBuf,

    #[command(subcommand)]
    transport: Transport,
}

#[derive(Subcommand)]
enum Transport {
    /// Use stdio transport
    Stdio,

    /// Use SSE transport
    Sse {
        /// Server address for SSE transport (e.g. 127.0.0.1:8090)
        #[arg(long, short, default_value = "127.0.0.1:8090")]
        endpoint: String,
    },

    /// Use WebSocket transport
    Ws {
        /// Server address for WebSocket transport (e.g. 127.0.0.1:9090)
        #[arg(long, short, default_value = "127.0.0.1:9090")]
        endpoint: String,
    },
}

struct McpServer {
    resources: Vec<Box<dyn ResourceReadHandler>>,
    prompts: Vec<Box<dyn PromptGetHandler>>,
}

impl ModelContextProtocolServer for McpServer {
    fn new() -> Self {
        Self { resources: vec![Box::new(resources::readme::Readme)], prompts: vec![Box::new(prompts::greet::Greet)] }
    }

    fn get_capabilities(&self) -> ServerCapabilities {
        let caps = ServerCapabilities {
            tools: Some(ServerCapabilitiesPromptsResourcesTools { list_changed: Some(false) }),
            resources: Some(ServerCapabilitiesPromptsResources { list_changed: Some(false), subscribe: Some(false) }),
            prompts: Some(ServerCapabilitiesPrompts { list_changed: Some(false) }),
            ..Default::default()
        };

        caps
    }

    fn get_resources(&self) -> &Vec<Box<dyn ResourceReadHandler>> {
        &self.resources
    }

    fn get_prompts(&self) -> &Vec<Box<dyn PromptGetHandler>> {
        &self.prompts
    }

    fn create_tools(&self) -> Vec<Box<dyn ToolCallHandler>> {
        vec![
            Box::new(tools::echo::Echo),
            Box::new(tools::memory::Memory),
            Box::new(tools::fetch::Fetch::default()),
            Box::new(tools::random::RandomNumber),
        ]
    }
}

fn setup_logging(log_path: PathBuf) -> Result<()> {
    // Create parent directory if it doesn't exist
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).context("Failed to create log directory")?;
    }

    // Create file appender
    let file_appender = RollingFileAppender::new(
        Rotation::NEVER,
        log_path.parent().unwrap_or(&PathBuf::from(".")),
        log_path.file_name().unwrap_or_default(),
    );

    // Initialize tracing subscriber with cleaner formatting
    tracing_subscriber::fmt()
        .with_timer(tracing_subscriber::fmt::time::UtcTime::rfc_3339())
        .with_level(true)
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .with_ansi(false) // Disable ANSI color codes
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

    let transport = match &args.transport {
        Transport::Stdio => TransportConfig::Stdio(StdioConfig {}),
        Transport::Sse { endpoint } => TransportConfig::Sse(SseConfig::builder().endpoint(endpoint.clone()).build()),
        Transport::Ws { endpoint } => TransportConfig::Ws(WsConfig::builder().endpoint(endpoint.clone()).build()),
    };

    bioma_tool::server::start::<McpServer>("mcp_server", transport).await
}
