use anyhow::{Error, Result};
use bioma_actor::{Engine, EngineOptions};
use bioma_mcp::{
    resources::ResourceReadHandler,
    schema::{
        ServerCapabilities, ServerCapabilitiesPrompts, ServerCapabilitiesPromptsResources,
        ServerCapabilitiesPromptsResourcesTools,
    },
    server::{
        Context, ModelContextProtocolServer, Pagination, ResponseType, Server, SseConfig, StdioConfig,
        StreamableConfig, TransportConfig, WsConfig,
    },
    tools::ToolCallHandler,
};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::signal;
use tracing::{error, info};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::prelude::*;

mod tools;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, short, default_value = "rag_mcp_server.log")]
    log_file: PathBuf,

    #[arg(long, short, default_value = ".")]
    base_dir: PathBuf,

    #[arg(long, short, default_value = "20")]
    page_size: usize,

    #[arg(long, default_value = "ws://0.0.0.0:9123")]
    endpoint: String,

    #[arg(long, default_value = "dev")]
    namespace: String,

    #[arg(long, default_value = "bioma")]
    database: String,

    #[arg(long, default_value = "root")]
    username: String,

    #[arg(long, default_value = "root")]
    password: String,

    #[command(subcommand)]
    transport: Option<Transport>,
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

    Streamable {
        #[arg(long, short, default_value = "127.0.0.1:7090")]
        endpoint: String,

        #[arg(long, default_value = "json")]
        response_type: String,

        #[arg(long, default_value = "0.0.0.0", value_delimiter = ',')]
        allowed_origins: Vec<String>,
    },
}

#[derive(Clone)]
pub struct RagMcpServer {
    transport_config: TransportConfig,
    capabilities: ServerCapabilities,
    pagination: Pagination,
    log_path: PathBuf,
    engine: Arc<Engine>,
}

impl ModelContextProtocolServer for RagMcpServer {
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
        let file_appender = RollingFileAppender::new(
            Rotation::NEVER,
            self.log_path.parent().unwrap_or(&PathBuf::from(".")),
            self.log_path.file_name().unwrap_or_default(),
        );

        let filter = std::env::var("RUST_LOG")
            .map(|val| tracing_subscriber::EnvFilter::new(val))
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug"));

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
            .with_filter(filter);

        Some(Box::new(file_layer))
    }

    async fn new_resources(&self, _context: Context) -> Vec<Arc<dyn ResourceReadHandler>> {
        vec![]
    }

    async fn new_prompts(&self, _context: Context) -> Vec<Arc<dyn bioma_mcp::prompts::PromptGetHandler>> {
        vec![]
    }

    async fn new_tools(&self, context: Context) -> Vec<Arc<dyn ToolCallHandler>> {
        let engine = &self.engine;
        let mut tools: Vec<Arc<dyn ToolCallHandler>> = Vec::new();

        if let Ok(index_tool) = tools::index::IndexTool::new(engine).await {
            tools.push(Arc::new(index_tool));
        }

        if let Ok(retrieve_tool) = tools::retrieve::RetrieveTool::new(engine).await {
            tools.push(Arc::new(retrieve_tool));
        }

        if let Ok(embed_tool) = tools::embed::EmbedTool::new(engine).await {
            tools.push(Arc::new(embed_tool));
        }

        if let Ok(rerank_tool) = tools::rerank::RerankTool::new(engine).await {
            tools.push(Arc::new(rerank_tool));
        }

        let ingest_tool = tools::ingest::IngestTool::new(engine.clone());
        tools.push(Arc::new(ingest_tool));

        if let Ok(sources_tool) = tools::sources::SourcesTool::new(engine).await {
            tools.push(Arc::new(sources_tool));
        }

        if let Ok(delete_tool) = tools::delete::DeleteTool::new(engine).await {
            tools.push(Arc::new(delete_tool));
        }

        let generate_tool = tools::generate::GenerateTool::new(engine, context);
        tools.push(Arc::new(generate_tool));

        tools
    }

    async fn on_error(&self, error: Error) {
        error!("Error: {}", error);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let transport_config = match &args.transport {
        Some(transport) => match transport {
            Transport::Stdio => TransportConfig::Stdio(StdioConfig {}),
            Transport::Sse { endpoint } => {
                TransportConfig::Sse(SseConfig::builder().endpoint(endpoint.clone()).build())
            }
            Transport::Ws { endpoint } => TransportConfig::Ws(WsConfig::builder().endpoint(endpoint.clone()).build()),
            Transport::Streamable { endpoint, response_type, allowed_origins } => {
                let response_type = match response_type.to_lowercase().as_str() {
                    "sse" => ResponseType::SSE,
                    "json" => ResponseType::Json,
                    _ => ResponseType::Json,
                };

                TransportConfig::Streamable(
                    StreamableConfig::builder()
                        .endpoint(endpoint.clone())
                        .response_type(response_type)
                        .allowed_origins(allowed_origins.clone())
                        .build(),
                )
            }
        },
        None => TransportConfig::Stdio(StdioConfig {}),
    };

    let capabilities = ServerCapabilities {
        tools: Some(ServerCapabilitiesPromptsResourcesTools { list_changed: Some(false) }),
        resources: Some(ServerCapabilitiesPromptsResources { list_changed: Some(true), subscribe: Some(true) }),
        prompts: Some(ServerCapabilitiesPrompts { list_changed: Some(false) }),
        completions: Some(std::collections::BTreeMap::new()),
        logging: Some(std::collections::BTreeMap::new()),
        ..Default::default()
    };

    let engine_options = EngineOptions::builder()
        .endpoint(args.endpoint.into())
        .namespace(args.namespace.into())
        .database(args.database.into())
        .username(args.username.into())
        .password(args.password.into())
        .build();

    println!("Connecting to engine...");
    let engine = Arc::new(Engine::connect(engine_options).await?);

    let server = RagMcpServer {
        transport_config,
        capabilities,
        pagination: Pagination::new(args.page_size),
        log_path: args.log_file,
        engine,
    };

    let mcp_server = Server::new(server);

    let signal_task = tokio::spawn(async {
        signal::ctrl_c().await.expect("Failed to install CTRL+C signal handler");
        info!("Received shutdown signal, exiting...");
        std::process::exit(0);
    });

    let server_task = tokio::spawn(async move { mcp_server.start().await });

    tokio::select! {
        _ = signal_task => {},
        _ = server_task => {},
    }

    Ok(())
}
