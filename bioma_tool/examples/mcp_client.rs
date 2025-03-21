use anyhow::Result;
use bioma_tool::{
    client::{ModelContextProtocolClient, ServerConfig, SseConfig, StdioConfig, TransportConfig, WsConfig},
    schema::{CallToolRequestParams, Implementation, ReadResourceRequestParams},
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
    /// Use stdio transport
    Stdio {
        /// Path to the MCP server executable
        command: String,

        /// Args to pass to the MCP server
        #[arg(num_args = 0.., value_delimiter = ' ')]
        args: Option<Vec<String>>,
    },
    /// Use SSE transport
    Sse {
        /// Server URL (e.g. http://127.0.0.1:8090)
        #[arg(long, short, default_value = "http://127.0.0.1:8090")]
        endpoint: String,
    },
    /// Use WebSocket transport
    Ws {
        /// WebSocket server URL (e.g. ws://127.0.0.1:9090)
        #[arg(long, short, default_value = "ws://127.0.0.1:9090")]
        endpoint: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    info!("Starting MCP client...");
    let args = Args::parse();

    // Configure and start the MCP server process
    info!("Starting MCP server process...");

    let server = match &args.transport {
        Transport::Stdio { command, args } => ServerConfig::builder()
            .name("bioma-tool".to_string())
            .transport(TransportConfig::Stdio(StdioConfig {
                command: command.clone(),
                args: args.clone().unwrap_or_default(),
            }))
            .build(),
        Transport::Sse { endpoint } => ServerConfig::builder()
            .name(endpoint.clone())
            .transport(TransportConfig::Sse(SseConfig::builder().endpoint(endpoint.clone()).build()))
            .build(),
        Transport::Ws { endpoint } => ServerConfig::builder()
            .name(endpoint.clone())
            .transport(TransportConfig::Ws(WsConfig { endpoint: endpoint.clone() }))
            .build(),
    };

    // Create client
    let mut client = ModelContextProtocolClient::new(server).await?;

    // Initialize the client
    info!("Initializing client...");
    let init_result = client
        .initialize(Implementation { name: "mcp_client_example".to_string(), version: "0.1.0".to_string() })
        .await?;
    info!("Server capabilities: {:?}", init_result.capabilities);

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Notify the server that the client has initialized
    client.initialized().await?;

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // List prompts
    info!("Listing prompts...");
    let prompts_result = client.list_prompts(None).await;
    match prompts_result {
        Ok(prompts_result) => info!("Available prompts: {:?}", prompts_result.prompts),
        Err(e) => error!("Error listing prompts: {:?}", e),
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // List resources
    info!("Listing resources...");
    let resources_result = client.list_resources(None).await;
    match resources_result {
        Ok(resources_result) => {
            info!("Available resources: {:?}", resources_result.resources);

            // Look for the filesystem resource and read files from it
            if let Some(filesystem) = resources_result.resources.iter().find(|r| r.name == "filesystem") {
                info!("Found filesystem resource: {}", filesystem.uri);

                // Read README.md file
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

                // Then read the root directory
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

                // Try to subscribe to the directory if subscription is supported
                info!("Checking for resource templates...");
                let templates_result = client.list_resource_templates(None).await;
                match templates_result {
                    Ok(templates) => {
                        info!("Found {} resource templates", templates.resource_templates.len());
                        for template in templates.resource_templates {
                            info!("- Template: {} URI: {}", template.name, template.uri_template);
                        }

                        info!("Trying to subscribe to filesystem changes...");
                        // Use the root URI for subscription
                        // TODO: Do this later, rn since mcp log is inclued it sends notifications too often and makes IDE crash.
                        // let subscription_uri = "file:///";

                        // match client.subscribe_resource(subscription_uri.to_string()).await {
                        //     Ok(_) => info!("Successfully subscribed to filesystem changes"),
                        //     Err(e) => info!("Subscription not supported or failed: {:?}", e),
                        // }

                        // Later, you could unsubscribe like this:
                        // match client.unsubscribe_resource(subscription_uri.to_string()).await {
                        //     Ok(_) => info!("Successfully unsubscribed from filesystem changes"),
                        //     Err(e) => info!("Unsubscription failed: {:?}", e),
                        // }
                    }
                    Err(e) => info!("Resource templates not supported: {:?}", e),
                }
            } else {
                info!("Filesystem resource not found, falling back to readme resource");

                // Use the first available resource (likely the readme from before)
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

    // List tools
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

    // Make an echo tool call
    info!("Making echo tool call...");
    let echo_args = serde_json::json!({
        "message": "Hello from MCP client!"
    });
    let echo_args =
        CallToolRequestParams { name: "echo".to_string(), arguments: serde_json::from_value(echo_args).unwrap() };
    let echo_result = client.call_tool(echo_args).await?;
    info!("Echo response: {:?}", echo_result);

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Shutdown the client
    info!("Shutting down client...");
    client.close().await?;

    Ok(())
}
