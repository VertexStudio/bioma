use anyhow::Result;
use bioma_tool::{
    client::{ModelContextProtocolClient, ServerConfig, SseConfig, TransportConfig},
    schema::{CallToolRequestParams, Implementation, ReadResourceRequestParams},
};
use clap::Parser;
use tracing::{error, info};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Server URL (e.g. http://127.0.0.1:8090)
    #[arg(long, default_value = "http://127.0.0.1:8090")]
    endpoint: String,
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

    let server = ServerConfig::builder()
        .name(args.endpoint.to_string())
        .transport(TransportConfig::Sse(SseConfig::builder().endpoint(args.endpoint).build()))
        .build();

    // Create client
    let mut client = ModelContextProtocolClient::new(server).await?;

    // Wait to open SSE connection
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

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

    // Make a list_directory tool call
    info!("Making list_directory tool call...");
    let tree_args = serde_json::json!({
        "path": "/data/BiomaAI"
    });
    let tree_args = CallToolRequestParams {
        name: "list_directory".to_string(),
        arguments: Some(serde_json::from_value(tree_args).unwrap()),
    };
    let tree_result = client.call_tool(tree_args).await;
    match tree_result {
        Ok(result) => info!("list_directory response: {:?}", result),
        Err(e) => error!("Error calling list_directory: {:?}", e),
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Shutdown the client
    info!("Shutting down client...");

    Ok(())
}
