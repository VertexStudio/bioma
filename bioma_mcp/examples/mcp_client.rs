use std::collections::HashMap;

use anyhow::Result;
use bioma_mcp::{
    client::{Client, ModelContextProtocolClient, ServerConfig, SseConfig, StdioConfig, TransportConfig, WsConfig},
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

    #[arg(long, short, default_value = "1")]
    clients: usize,
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
pub struct ExampleMcpClient {
    server_config: ServerConfig,
    capabilities: ClientCapabilities,
    roots: Vec<Root>,
}

impl ModelContextProtocolClient for ExampleMcpClient {
    async fn get_server_config(&self) -> ServerConfig {
        self.server_config.clone()
    }

    async fn get_capabilities(&self) -> ClientCapabilities {
        self.capabilities.clone()
    }

    async fn get_roots(&self) -> Vec<Root> {
        self.roots.clone()
    }

    async fn on_create_message(&self, _params: CreateMessageRequestParams) -> CreateMessageResult {
        todo!()
    }
}

async fn run_client(client_id: usize, server_config: ServerConfig) -> Result<()> {
    info!("Client {}: Starting...", client_id);

    let capabilities =
        ClientCapabilities { roots: Some(ClientCapabilitiesRoots { list_changed: Some(true) }), ..Default::default() };

    let client = ExampleMcpClient {
        server_config,
        capabilities,
        roots: vec![Root {
            name: Some(format!("workspace-{}", client_id)),
            uri: format!("file:///workspace-{}", client_id),
        }],
    };

    let mut client = Client::new(client).await?;

    info!("Client {}: Initializing...", client_id);
    let init_result = client
        .initialize(Implementation { name: format!("mcp_client_example-{}", client_id), version: "0.1.0".to_string() })
        .await?;
    info!("Client {}: Server capabilities: {:?}", client_id, init_result.capabilities);

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    client.initialized().await?;

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    info!("Client {}: Listing prompts...", client_id);
    let prompts_result = client.list_prompts(None).await;
    match prompts_result {
        Ok(prompts_result) => info!("Client {}: Available prompts: {:?}", client_id, prompts_result.prompts),
        Err(e) => error!("Client {}: Error listing prompts: {:?}", client_id, e),
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    info!("Client {}: Listing resources...", client_id);
    let resources_result = client.list_resources(None).await;
    match resources_result {
        Ok(resources_result) => {
            info!("Client {}: Available resources: {:?}", client_id, resources_result.resources);

            if let Some(filesystem) = resources_result.resources.iter().find(|r| r.name == "filesystem") {
                info!("Client {}: Found filesystem resource: {}", client_id, filesystem.uri);

                let readme_uri = "file:///bioma/README.md";
                info!("Client {}: Reading file: {}", client_id, readme_uri);

                let readme_result =
                    client.read_resource(ReadResourceRequestParams { uri: readme_uri.to_string() }).await;
                match readme_result {
                    Ok(result) => {
                        if let Some(content) = result.contents.first() {
                            if let Some(text) = content.get("text").and_then(|t| t.as_str()) {
                                info!(
                                    "Client {}: README.md content preview (first 100 chars): {}",
                                    client_id,
                                    text.chars().take(100).collect::<String>()
                                );
                            } else if let Some(blob) = content.get("blob").and_then(|b| b.as_str()) {
                                info!("Client {}: README.md is a binary file with {} bytes", client_id, blob.len());
                            }
                        }
                    }
                    Err(e) => error!("Client {}: Error reading README.md: {:?}", client_id, e),
                }

                let dir_uri = "file:///";
                info!("Client {}: Reading directory: {}", client_id, dir_uri);

                let dir_result = client.read_resource(ReadResourceRequestParams { uri: dir_uri.to_string() }).await;
                match dir_result {
                    Ok(result) => {
                        info!("Client {}: Directory contents:", client_id);
                        for content in result.contents {
                            if let Some(text) = content.get("text").and_then(|t| t.as_str()) {
                                info!("Client {}: - {}", client_id, text);
                            }
                        }
                    }
                    Err(e) => error!("Client {}: Error reading root directory: {:?}", client_id, e),
                }

                info!("Client {}: Checking for resource templates...", client_id);
                let templates_result = client.list_resource_templates(None).await;
                match templates_result {
                    Ok(templates) => {
                        info!("Client {}: Found {} resource templates", client_id, templates.resource_templates.len());
                        for template in templates.resource_templates {
                            info!("Client {}: - Template: {} URI: {}", client_id, template.name, template.uri_template);
                        }

                        info!("Client {}: Trying to subscribe to filesystem changes...", client_id);
                    }
                    Err(e) => info!("Client {}: Resource templates not supported: {:?}", client_id, e),
                }
            } else {
                info!("Client {}: Filesystem resource not found, falling back to readme resource", client_id);

                if !resources_result.resources.is_empty() {
                    let read_result = client
                        .read_resource(ReadResourceRequestParams { uri: resources_result.resources[0].uri.clone() })
                        .await;

                    match read_result {
                        Ok(result) => info!("Client {}: Resource content: {:?}", client_id, result),
                        Err(e) => error!("Client {}: Error reading resource: {:?}", client_id, e),
                    }
                }
            }
        }
        Err(e) => error!("Client {}: Error listing resources: {:?}", client_id, e),
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    info!("Client {}: Listing tools...", client_id);
    let tools_result = client.list_tools(None).await;
    match tools_result {
        Ok(tools_result) => {
            info!("Client {}: Available tools:", client_id);
            for tool in tools_result.tools {
                info!("Client {}: - {}", client_id, tool.name);
            }
        }
        Err(e) => error!("Client {}: Error listing tools: {:?}", client_id, e),
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    info!("Client {}: Making echo tool call...", client_id);
    let echo_args = serde_json::json!({
        "message": format!("Hello from MCP client {}!", client_id)
    });
    let echo_args =
        CallToolRequestParams { name: "echo".to_string(), arguments: serde_json::from_value(echo_args).unwrap() };
    let echo_result = client.call_tool(echo_args).await?;
    info!("Client {}: Echo response: {:?}", client_id, echo_result);

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    info!("Client {}: Updating roots...", client_id);
    let roots = HashMap::from([(
        format!("workspace-{}", client_id),
        Root { name: Some(format!("workspace-{}", client_id)), uri: format!("file:///workspace-{}", client_id) },
    )]);
    client.update_roots(roots).await?;

    info!("Client {}: Shutting down...", client_id);
    client.close().await?;

    info!("Client {}: Completed successfully", client_id);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    info!("Starting MCP client manager...");
    let args = Args::parse();

    let server_config = match &args.transport {
        Transport::Stdio { command, args } => {
            info!("Starting MCP server process with command: {}", command);
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

    info!("Spawning {} client connections...", args.clients);

    let mut handles = Vec::with_capacity(args.clients);
    for i in 0..args.clients {
        let server_config_clone = server_config.clone();
        let handle = tokio::spawn(async move {
            if let Err(e) = run_client(i, server_config_clone).await {
                error!("Client {} error: {:?}", i, e);
            }
        });
        handles.push(handle);
    }

    for (i, handle) in handles.into_iter().enumerate() {
        match handle.await {
            Ok(_) => info!("Client {} task completed", i),
            Err(e) => error!("Client {} task join error: {:?}", i, e),
        }
    }

    info!("All clients finished, exiting");
    Ok(())
}
