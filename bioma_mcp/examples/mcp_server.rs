use anyhow::{Error, Result};
use bioma_mcp::{
    prompts::{self, PromptGetHandler},
    resources::{self, ResourceReadHandler},
    schema::{
        ServerCapabilities, ServerCapabilitiesPrompts, ServerCapabilitiesPromptsResources,
        ServerCapabilitiesPromptsResourcesTools,
    },
    server::{
        Context, ModelContextProtocolServer, Pagination, Server, SseConfig, StdioConfig, TransportConfig, WsConfig,
    },
    tools::{self, ToolCallHandler},
};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::error;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::prelude::*;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, short, default_value = "mcp_server.log")]
    log_file: PathBuf,

    #[arg(long, short, default_value = ".")]
    base_dir: PathBuf,

    #[arg(long, short, default_value = "20")]
    page_size: usize,

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

// fn setup_logging(log_path: PathBuf) -> Result<()> {
//     if let Some(parent) = log_path.parent() {
//         std::fs::create_dir_all(parent).context("Failed to create log directory")?;
//     }

//     let file_appender = RollingFileAppender::new(
//         Rotation::NEVER,
//         log_path.parent().unwrap_or(&PathBuf::from(".")),
//         log_path.file_name().unwrap_or_default(),
//     );

//     tracing_subscriber::fmt()
//         .with_timer(tracing_subscriber::fmt::time::UtcTime::rfc_3339())
//         .with_level(true)
//         .with_target(true)
//         .with_thread_ids(true)
//         .with_file(true)
//         .with_line_number(true)
//         .with_ansi(false)
//         .with_span_events(FmtSpan::CLOSE)
//         .with_writer(file_appender)
//         .with_max_level(Level::DEBUG)
//         .init();

//     info!("Logging system initialized");

//     Ok(())
// }

#[derive(Clone)]
pub struct ExampleMcpServer {
    transport_config: TransportConfig,
    capabilities: ServerCapabilities,
    base_dir: PathBuf,
    pagination: Pagination,
    log_path: PathBuf,
}

impl ModelContextProtocolServer for ExampleMcpServer {
    async fn get_transport_config(&self) -> TransportConfig {
        self.transport_config.clone()
    }

    async fn get_capabilities(&self) -> ServerCapabilities {
        self.capabilities.clone()
    }

    async fn get_pagination(&self) -> Option<Pagination> {
        Some(self.pagination.clone())
    }

    async fn get_tracing_layer(&self) -> Option<bioma_mcp::server::TracingLayer> {
        // Create file appender
        let file_appender = RollingFileAppender::new(
            Rotation::NEVER,
            self.log_path.parent().unwrap_or(&PathBuf::from(".")),
            self.log_path.file_name().unwrap_or_default(),
        );

        // Create the file layer
        let file_layer = tracing_subscriber::fmt::layer()
            .with_timer(tracing_subscriber::fmt::time::UtcTime::rfc_3339())
            .with_level(true)
            .with_target(true)
            .with_thread_ids(true)
            .with_file(true)
            .with_line_number(true)
            .with_ansi(false)
            .with_span_events(FmtSpan::CLOSE)
            .with_writer(file_appender)
            .with_filter(tracing_subscriber::filter::LevelFilter::DEBUG);

        // Box the layer and return it
        Some(Box::new(file_layer))
    }

    async fn new_resources(&self, context: Context) -> Vec<Arc<dyn ResourceReadHandler>> {
        vec![
            Arc::new(resources::readme::Readme),
            Arc::new(resources::filesystem::FileSystem::new(self.base_dir.clone(), context)),
        ]
    }

    async fn new_prompts(&self, _context: Context) -> Vec<Arc<dyn PromptGetHandler>> {
        vec![Arc::new(prompts::greet::Greet)]
    }

    async fn new_tools(&self, context: Context) -> Vec<Arc<dyn ToolCallHandler>> {
        vec![
            Arc::new(tools::echo::Echo),
            Arc::new(tools::memory::Memory),
            Arc::new(tools::fetch::Fetch::default()),
            Arc::new(tools::random::RandomNumber),
            Arc::new(tools::workflow::Workflow::new(true, None)),
            Arc::new(tools::sampling::Sampling::new(context)),
        ]
    }

    async fn on_error(&self, error: Error) {
        error!("Error: {}", error);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let transport_config = match &args.transport {
        Transport::Stdio => TransportConfig::Stdio(StdioConfig {}),
        Transport::Sse { endpoint } => TransportConfig::Sse(SseConfig::builder().endpoint(endpoint.clone()).build()),
        Transport::Ws { endpoint } => TransportConfig::Ws(WsConfig::builder().endpoint(endpoint.clone()).build()),
    };

    let capabilities = ServerCapabilities {
        tools: Some(ServerCapabilitiesPromptsResourcesTools { list_changed: Some(false) }),
        resources: Some(ServerCapabilitiesPromptsResources { list_changed: Some(true), subscribe: Some(true) }),
        prompts: Some(ServerCapabilitiesPrompts { list_changed: Some(false) }),
        completions: Some(std::collections::BTreeMap::new()),
        logging: Some(std::collections::BTreeMap::new()),
        ..Default::default()
    };

    let server = ExampleMcpServer {
        transport_config,
        capabilities,
        base_dir: args.base_dir,
        pagination: Pagination::new(args.page_size),
        log_path: args.log_file,
    };

    let mcp_server = Server::new(server);

    let _ = mcp_server.start().await;

    Ok(())
}
