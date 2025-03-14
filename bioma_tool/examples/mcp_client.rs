use anyhow::Result;
use bioma_tool::{
    client::{ModelContextProtocolClient, ServerConfig, StdioConfig, TransportConfig},
    schema::{CallToolRequestParams, Implementation, ReadResourceRequestParams},
};
use clap::Parser;
use tracing::{error, info};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the MCP server executable
    command: String,

    /// Args to pass to the MCP server
    #[arg(num_args = 0.., value_delimiter = ' ')]
    args: Option<Vec<String>>,
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

    let server = ServerConfig {
        name: "bioma-tool".to_string(),
        transport: TransportConfig::Stdio(StdioConfig { command: args.command, args: args.args.unwrap_or_default() }),
        request_timeout: 5,
        version: "0.1.0".to_string(),
        enabled: true,
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

            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

            // Read resource
            if resources_result.resources.len() > 0 {
                info!("Reading resource...");
                let resource_result = client
                    .read_resource(ReadResourceRequestParams { uri: resources_result.resources[0].uri.clone() })
                    .await?;
                info!("Resource content: {:?}", resource_result.contents);
            }
        }
        Err(e) => error!("Error listing resources: {:?}", e),
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // List tools
    info!("Listing tools...");
    let tools_result = client.list_tools(None).await;
    match tools_result {
        Ok(tools_result) => info!("Available tools: {:#?}", tools_result.tools),
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
