use anyhow::{Context, Result};
use bioma_tool::{
    schema::{
        Prompt, PromptArgument, Resource, ServerCapabilities, ServerCapabilitiesPrompts,
        ServerCapabilitiesPromptsResources, ServerCapabilitiesPromptsResourcesTools,
    },
    tools::{self, ToolCallHandler},
    transport::{StdioTransport, TransportType, WebSocketTransport},
    ModelContextProtocolServer,
};
use clap::Parser;
use std::path::PathBuf;
use tracing::{info, Level};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::format::FmtSpan;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the log file
    #[arg(long, default_value = "mcp_server.log")]
    log_file: PathBuf,

    /// Transport type (stdio or websocket)
    #[arg(long, default_value = "stdio")]
    transport: String,

    /// WebSocket address (only used with websocket transport)
    #[arg(long, default_value = "127.0.0.1:8080")]
    ws_addr: String,
}

struct McpServer {
    tools: Vec<Box<dyn ToolCallHandler>>,
    resources: Vec<Resource>,
    prompts: Vec<Prompt>,
}

impl ModelContextProtocolServer for McpServer {
    fn new() -> Self {
        let example_resource = Resource {
            name: "example.txt".to_string(),
            uri: "file:///example.txt".to_string(),
            description: Some("An example text file".to_string()),
            mime_type: Some("text/plain".to_string()),
            annotations: None,
        };

        let example_prompt = Prompt {
            name: "greet".to_string(),
            description: Some("A friendly greeting prompt".to_string()),
            arguments: Some(vec![PromptArgument {
                name: "name".to_string(),
                description: Some("Name of the person to greet".to_string()),
                required: Some(true),
            }]),
        };

        Self {
            tools: vec![
                Box::new(tools::echo::Echo),
                Box::new(tools::memory::Memory),
                Box::new(tools::fetch::Fetch::default()),
            ],
            resources: vec![example_resource],
            prompts: vec![example_prompt],
        }
    }

    fn get_capabilities(&self) -> ServerCapabilities {
        ServerCapabilities {
            tools: Some(ServerCapabilitiesPromptsResourcesTools {
                list_changed: Some(false),
            }),
            resources: Some(ServerCapabilitiesPromptsResources {
                list_changed: Some(false),
                subscribe: Some(false),
            }),
            prompts: Some(ServerCapabilitiesPrompts {
                list_changed: Some(false),
            }),
            ..Default::default()
        }
    }

    fn get_resources(&self) -> &Vec<Resource> {
        &self.resources
    }

    fn get_prompts(&self) -> &Vec<Prompt> {
        &self.prompts
    }

    fn get_tools(&self) -> &Vec<Box<dyn ToolCallHandler>> {
        &self.tools
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

    let transport = match args.transport.as_str() {
        "stdio" => TransportType::Stdio(StdioTransport::new()),
        "websocket" => TransportType::WebSocket(WebSocketTransport::new(args.ws_addr)),
        _ => return Err(anyhow::anyhow!("Invalid transport type")),
    };

    bioma_tool::start_server::<McpServer>(transport).await
}
