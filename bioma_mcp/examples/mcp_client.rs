use anyhow::Result;
use bioma_mcp::{
    client::{
        Client, Metadata, ModelContextProtocolClient, ServerConfig, SseConfig, StdioConfig, TransportConfig, WsConfig,
    },
    schema::{
        CallToolRequestParams, ClientCapabilities, ClientCapabilitiesRoots, CreateMessageRequestParams,
        CreateMessageResult, Implementation, ReadResourceRequestParams, Root,
    },
};
use clap::{Parser, Subcommand};
use tracing::{error, info};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    transport: Transport,
}

#[derive(Subcommand)]
enum Transport {
    Stdio {
        command: String,

        #[arg(num_args = 0.., value_delimiter = ' ')]
        args: Option<Vec<String>>,
    },

    Sse {
        #[arg(long, short, default_value = "http://127.0.0.1:8090")]
        endpoint: String,
    },

    Ws {
        #[arg(long, short, default_value = "ws://127.0.0.1:9090")]
        endpoint: String,
    },
}

#[derive(Clone)]
pub struct ClientMetadata;

impl Metadata for ClientMetadata {}

struct ExampleMcpClient {
    server_config: ServerConfig,
    capabilities: ClientCapabilities,
    roots: Vec<Root>,
}

impl ModelContextProtocolClient<ClientMetadata> for ExampleMcpClient {
    async fn get_server_config(&self) -> ServerConfig {
        self.server_config.clone()
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
        _meta: ClientMetadata,
    ) -> CreateMessageResult {
        todo!()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    info!("Starting MCP client...");
    let args = Args::parse();

    let server = match &args.transport {
        Transport::Stdio { command, args } => {
            info!("Starting to MCP server process with command: {}", command);
            ServerConfig::builder()
                .name("bioma-tool".to_string())
                .transport(TransportConfig::Stdio(StdioConfig {
                    command: command.clone(),
                    args: args.clone().unwrap_or_default(),
                }))
                .build()
        }
        Transport::Sse { endpoint } => {
            info!("Connecting to MCP server at {}", endpoint);
            ServerConfig::builder()
                .name("bioma-tool".to_string())
                .transport(TransportConfig::Sse(SseConfig::builder().endpoint(endpoint.clone()).build()))
                .build()
        }
        Transport::Ws { endpoint } => {
            info!("Connecting to MCP server at {}", endpoint);
            ServerConfig::builder()
                .name("bioma-tool".to_string())
                .transport(TransportConfig::Ws(WsConfig { endpoint: endpoint.clone() }))
                .build()
        }
    };

    let capabilities =
        ClientCapabilities { roots: Some(ClientCapabilitiesRoots { list_changed: Some(true) }), ..Default::default() };

    let client = ExampleMcpClient {
        server_config: server,
        capabilities,
        roots: vec![Root { name: Some("workspace".to_string()), uri: "file:///workspace".to_string() }],
    };

    let mut client = Client::new(client).await?;

    info!("Initializing client...");
    let init_result = client
        .initialize(Implementation { name: "mcp_client_example".to_string(), version: "0.1.0".to_string() })
        .await?;
    info!("Server capabilities: {:?}", init_result.capabilities);

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    client.initialized().await?;

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    info!("Listing prompts...");
    let prompts_result = client.list_prompts(None).await;
    match prompts_result {
        Ok(prompts_result) => info!("Available prompts: {:?}", prompts_result.prompts),
        Err(e) => error!("Error listing prompts: {:?}", e),
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    info!("Listing resources...");
    let resources_result = client.list_resources(None).await;
    match resources_result {
        Ok(resources_result) => {
            info!("Available resources: {:?}", resources_result.resources);

            if let Some(filesystem) = resources_result.resources.iter().find(|r| r.name == "filesystem") {
                info!("Found filesystem resource: {}", filesystem.uri);

                let readme_uri = "file:///bioma/README.md";
                info!("Reading file: {}", readme_uri);

                let readme_result =
                    client.read_resource(ReadResourceRequestParams { uri: readme_uri.to_string() }).await;
                match readme_result {
                    Ok(result) => {
                        if let Some(content) = result.contents.first() {
                            if let Some(text) = content.get("text").and_then(|t| t.as_str()) {
                                info!(
                                    "README.md content preview (first 100 chars): {}",
                                    text.chars().take(100).collect::<String>()
                                );
                            } else if let Some(blob) = content.get("blob").and_then(|b| b.as_str()) {
                                info!("README.md is a binary file with {} bytes", blob.len());
                            }
                        }
                    }
                    Err(e) => error!("Error reading README.md: {:?}", e),
                }

                let dir_uri = "file:///";
                info!("Reading directory: {}", dir_uri);

                let dir_result = client.read_resource(ReadResourceRequestParams { uri: dir_uri.to_string() }).await;
                match dir_result {
                    Ok(result) => {
                        info!("Directory contents:");
                        for content in result.contents {
                            if let Some(text) = content.get("text").and_then(|t| t.as_str()) {
                                info!("- {}", text);
                            }
                        }
                    }
                    Err(e) => error!("Error reading root directory: {:?}", e),
                }

                info!("Checking for resource templates...");
                let templates_result = client.list_resource_templates(None).await;
                match templates_result {
                    Ok(templates) => {
                        info!("Found {} resource templates", templates.resource_templates.len());
                        for template in templates.resource_templates {
                            info!("- Template: {} URI: {}", template.name, template.uri_template);
                        }

                        info!("Trying to subscribe to filesystem changes...");
                    }
                    Err(e) => info!("Resource templates not supported: {:?}", e),
                }
            } else {
                info!("Filesystem resource not found, falling back to readme resource");

                if !resources_result.resources.is_empty() {
                    let read_result = client
                        .read_resource(ReadResourceRequestParams { uri: resources_result.resources[0].uri.clone() })
                        .await;

                    match read_result {
                        Ok(result) => info!("Resource content: {:?}", result),
                        Err(e) => error!("Error reading resource: {:?}", e),
                    }
                }
            }
        }
        Err(e) => error!("Error listing resources: {:?}", e),
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    info!("Listing tools...");
    let tools_result = client.list_tools(None).await;
    match tools_result {
        Ok(tools_result) => {
            info!("Available tools:");
            for tool in tools_result.tools {
                info!("- {}", tool.name);
            }
        }
        Err(e) => error!("Error listing tools: {:?}", e),
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    info!("Making echo tool call...");
    let echo_args = serde_json::json!({
        "message": "Hello from MCP client!"
    });
    let echo_args =
        CallToolRequestParams { name: "echo".to_string(), arguments: serde_json::from_value(echo_args).unwrap() };
    let echo_result = client.call_tool(echo_args).await?;
    info!("Echo response: {:?}", echo_result);

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    info!("Shutting down client...");
    client.close().await?;

    Ok(())
}
