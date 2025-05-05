use anyhow::Result;
use bioma_mcp::{
    client::{Client, ClientError, ModelContextProtocolClient, ServerConfig, StdioConfig, TransportConfig},
    progress::Progress,
    schema::{
        CallToolRequestParams, ClientCapabilities, ClientCapabilitiesRoots, CreateMessageRequestParams,
        CreateMessageResult, Implementation, Root,
    },
};
use bioma_rag::{
    indexer::TextChunkConfig,
    prelude::{DeleteSource, Index as IndexArgs, IndexContent, RetrieveContext, RetrieveQuery},
};
use clap::{Parser, Subcommand};
use futures_util::StreamExt;
use rag_mcp::{
    embed::{EmbeddingsQueryArgs, ModelEmbed},
    ingest::IngestArgs,
    sources::ListSourcesArgs,
};
use serde_json::json;
use std::collections::HashMap;
use tracing::{error, info};

#[derive(Parser)]
#[command(author, version, about = "RAG MCP Client Example", long_about = None)]
struct Args {
    #[command(subcommand)]
    transport: TransportArg,
}

#[derive(Subcommand)]
enum TransportArg {
    Stdio {
        #[arg(long, short, default_value = "target/debug/examples/server")]
        command: String,

        #[arg(long, short, default_value = "rag-client")]
        name: String,

        #[arg(long, short, default_value = "30")]
        request_timeout: u64,
    },

    Sse {
        #[arg(long, short, default_value = "http://127.0.0.1:8090")]
        endpoint: String,

        #[arg(long, short, default_value = "rag-client")]
        name: String,

        #[arg(long, short, default_value = "30")]
        request_timeout: u64,
    },

    Ws {
        #[arg(long, short, default_value = "ws://127.0.0.1:9090")]
        endpoint: String,

        #[arg(long, short, default_value = "rag-client")]
        name: String,

        #[arg(long, short, default_value = "30")]
        request_timeout: u64,
    },

    Streamable {
        #[arg(long, short, default_value = "http://127.0.0.1:7090")]
        endpoint: String,

        #[arg(long, short, default_value = "rag-client")]
        name: String,

        #[arg(long, short, default_value = "30")]
        request_timeout: u64,
    },
}

#[derive(Clone)]
pub struct RagMcpClient {
    server_configs: Vec<ServerConfig>,
    capabilities: ClientCapabilities,
    roots: Vec<Root>,
}

impl ModelContextProtocolClient for RagMcpClient {
    async fn get_server_configs(&self) -> Vec<ServerConfig> {
        self.server_configs.clone()
    }

    async fn get_capabilities(&self) -> ClientCapabilities {
        self.capabilities.clone()
    }

    async fn get_roots(&self) -> Vec<Root> {
        self.roots.clone()
    }

    async fn on_create_message(
        &self,
        _params: CreateMessageRequestParams,
        _progress: Progress,
    ) -> Result<CreateMessageResult, ClientError> {
        Err(ClientError::Request("Message creation not supported".into()))
    }
}

async fn upload_and_index(client: &mut Client<RagMcpClient>) -> Result<(), ClientError> {
    info!("Demonstrating upload and index operations");

    // Create typed upload arguments
    let sample_text = "This is a sample document for RAG. It contains information about Rust programming.";
    let upload_args = IngestArgs { url: format!("data:text/plain,{}", sample_text), path: "sample.txt".to_string() };

    let upload_call = CallToolRequestParams {
        name: "upload".to_string(),
        arguments: serde_json::from_value(serde_json::to_value(upload_args).unwrap()).unwrap(),
    };

    info!("Uploading sample document...");
    let upload_result = client.call_tool(upload_call, false).await?.await?;
    info!("Upload response: {:?}", upload_result);

    // Create typed index arguments
    let index_args = IndexArgs {
        content: IndexContent::Globs(bioma_rag::indexer::GlobsContent {
            globs: vec!["sample.txt".into()],
            config: TextChunkConfig::default(),
        }),
        source: "example".into(),
        summarize: false,
    };

    let index_call = CallToolRequestParams {
        name: "index".to_string(),
        arguments: serde_json::from_value(serde_json::to_value(index_args).unwrap()).unwrap(),
    };

    info!("Indexing uploaded document...");
    let mut index_operation = client.call_tool(index_call, true).await?;

    // Handle progress stream
    let mut progress_stream = index_operation.recv();
    tokio::spawn(async move {
        while let Some(progress) = progress_stream.next().await {
            info!("Index progress: {:?}", progress);
        }
    });

    let index_result = index_operation.await?;
    info!("Index response: {:?}", index_result);

    Ok(())
}

async fn retrieval(client: &mut Client<RagMcpClient>) -> Result<(), ClientError> {
    info!("Demonstrating retrieval operations");

    // Create typed retrieve arguments
    let retrieve_args = RetrieveContext {
        query: RetrieveQuery::Text("Rust programming".into()),
        limit: 5,
        threshold: 0.0,
        sources: vec!["example".into()],
    };

    let retrieve_call = CallToolRequestParams {
        name: "retrieve".to_string(),
        arguments: serde_json::from_value(serde_json::to_value(retrieve_args).unwrap()).unwrap(),
    };

    info!("Retrieving documents...");
    let retrieve_result = client.call_tool(retrieve_call, false).await?.await?;
    info!("Retrieval response: {:?}", retrieve_result);

    // Create typed embedding arguments
    let embed_args =
        EmbeddingsQueryArgs { model: ModelEmbed::NomicEmbedTextV15, input: json!("Rust programming language") };

    let embed_call = CallToolRequestParams {
        name: "embed".to_string(),
        arguments: serde_json::from_value(serde_json::to_value(embed_args).unwrap()).unwrap(),
    };

    info!("Generating embedding...");
    let embed_result = client.call_tool(embed_call, false).await?.await?;
    info!("Embedding response: {:?}", embed_result);

    Ok(())
}

async fn sources_and_delete(client: &mut Client<RagMcpClient>) -> Result<(), ClientError> {
    info!("Demonstrating sources listing and deletion");

    // Use the proper type for sources listing
    let sources_args = ListSourcesArgs {};

    let sources_call = CallToolRequestParams {
        name: "sources".to_string(),
        arguments: serde_json::from_value(serde_json::to_value(sources_args).unwrap()).unwrap(),
    };

    info!("Listing sources...");
    let sources_result = client.call_tool(sources_call, false).await?.await?;
    info!("Sources response: {:?}", sources_result);

    // Create typed delete arguments
    let delete_args = DeleteSource { sources: vec!["sample.txt".into()], delete_from_disk: false };

    let delete_call = CallToolRequestParams {
        name: "delete".to_string(),
        arguments: serde_json::from_value(serde_json::to_value(delete_args).unwrap()).unwrap(),
    };

    info!("Deleting source...");
    let delete_result = client.call_tool(delete_call, false).await?.await?;
    info!("Deletion response: {:?}", delete_result);

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    info!("Starting RAG MCP client...");
    let args = Args::parse();

    let server_configs: Vec<ServerConfig> = match &args.transport {
        TransportArg::Stdio { command, name, request_timeout } => {
            info!("Using stdio transport");
            vec![
                ServerConfig::builder()
                    .name(name.clone())
                    .transport(TransportConfig::Stdio(StdioConfig {
                        command: command.clone(),
                        args: vec!["stdio".to_string()],
                        env: HashMap::new(),
                    }))
                    .request_timeout(*request_timeout)
                    .build(),
            ]
        }
        TransportArg::Sse { endpoint, name, request_timeout } => {
            info!("Using SSE transport with endpoint: {}", endpoint);
            vec![
                ServerConfig::builder()
                    .name(name.clone())
                    .transport(TransportConfig::Sse(
                        bioma_mcp::client::SseConfig::builder().endpoint(endpoint.clone()).build(),
                    ))
                    .request_timeout(*request_timeout)
                    .build(),
            ]
        }
        TransportArg::Ws { endpoint, name, request_timeout } => {
            info!("Using WebSocket transport with endpoint: {}", endpoint);
            vec![
                ServerConfig::builder()
                    .name(name.clone())
                    .transport(TransportConfig::Ws(
                        bioma_mcp::client::WsConfig::builder().endpoint(endpoint.clone()).build(),
                    ))
                    .request_timeout(*request_timeout)
                    .build(),
            ]
        }
        TransportArg::Streamable { endpoint, name, request_timeout } => {
            info!("Using Streamable transport with endpoint: {}", endpoint);
            vec![
                ServerConfig::builder()
                    .name(name.clone())
                    .transport(TransportConfig::Streamable(
                        bioma_mcp::client::StreamableConfig::builder().endpoint(endpoint.clone()).build(),
                    ))
                    .request_timeout(*request_timeout)
                    .build(),
            ]
        }
    };

    info!("Loaded {} server configurations", server_configs.len());
    for (i, server) in server_configs.iter().enumerate() {
        info!("Server {}: {}", i + 1, server.name);
    }

    let capabilities =
        ClientCapabilities { roots: Some(ClientCapabilitiesRoots { list_changed: Some(true) }), ..Default::default() };

    let client = RagMcpClient {
        server_configs,
        capabilities,
        roots: vec![Root { name: Some("workspace".to_string()), uri: "file:///workspace".to_string() }],
    };

    let mut client = Client::new(client).await?;

    info!("Initializing client...");

    let init_result = client
        .initialize(Implementation { name: "rag_mcp_client_example".to_string(), version: "0.1.0".to_string() })
        .await?;
    info!("Server capabilities: {:?}", init_result);

    client.initialized().await?;

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    info!("Listing available tools...");
    let tools_operation = client.list_all_tools(None).await?;
    let mut all_tools = Vec::new();

    match tools_operation.await {
        Ok(tools) => {
            all_tools = tools;
            info!("Available tools:");
            for tool in &all_tools {
                info!("- {}", tool.name);
            }
        }
        Err(e) => {
            error!("Error listing tools: {:?}", e);
        }
    }

    if all_tools.iter().any(|t| t.name == "upload") && all_tools.iter().any(|t| t.name == "index") {
        if let Err(e) = upload_and_index(&mut client).await {
            error!("Error during upload and index: {:?}", e);
        }
    }

    if all_tools.iter().any(|t| t.name == "retrieve") && all_tools.iter().any(|t| t.name == "embed") {
        if let Err(e) = retrieval(&mut client).await {
            error!("Error during retrieval: {:?}", e);
        }
    }

    if all_tools.iter().any(|t| t.name == "sources") && all_tools.iter().any(|t| t.name == "delete") {
        if let Err(e) = sources_and_delete(&mut client).await {
            error!("Error during sources and delete: {:?}", e);
        }
    }

    info!("Shutting down client...");
    client.close().await?;

    Ok(())
}
