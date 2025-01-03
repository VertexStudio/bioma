use anyhow::Result;
use bioma_tool::{
    client::Client,
    schema::{CallToolRequestParams, Implementation, ReadResourceRequestParams},
    transport::{
        client::{McpServer, StdioTransport},
        TransportType,
    },
};
use clap::Parser;
use tracing::info;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Transport type (stdio or websocket)
    #[arg(long, default_value = "stdio")]
    transport: String,

    /// WebSocket address if using websocket transport
    #[arg(long, default_value = "127.0.0.1:3000")]
    ws_addr: String,

    /// Path to the MCP server executable
    #[arg(long, default_value = "target/release/examples/mcp_server")]
    server_path: String,

    /// Log file path
    #[arg(long, default_value = ".output/mcp_server-bioma.log")]
    server_log_file: String,
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
    let server = McpServer {
        command: args.server_path,
        args: vec![
            "--transport".to_string(),
            args.transport.clone(),
            "--log-file".to_string(),
            args.server_log_file.clone(),
        ],
    };
    let transport = StdioTransport::new();
    transport.start_process(&server).await?;

    // Create transport and client
    let transport = match args.transport.as_str() {
        "stdio" => TransportType::ClientStdio(transport),
        "websocket" => return Err(anyhow::anyhow!("WebSocket transport not yet implemented for client")),
        _ => return Err(anyhow::anyhow!("Invalid transport type")),
    };
    let mut client = Client::new(transport).await?;

    // Initialize the client
    info!("Initializing client...");
    let init_result = client
        .initialize(Implementation { name: "mcp_client_example".to_string(), version: "0.1.0".to_string() })
        .await?;
    info!("Server capabilities: {:?}", init_result.capabilities);

    // List tools
    info!("Listing tools...");
    let tools_result = client.list_tools(None).await?;
    info!("Available tools: {:?}", tools_result.tools);

    // List prompts
    info!("Listing prompts...");
    let prompts_result = client.list_prompts(None).await?;
    info!("Available prompts: {:?}", prompts_result.prompts);

    // List resources
    info!("Listing resources...");
    let resources_result = client.list_resources(None).await?;
    info!("Available resources: {:?}", resources_result.resources);

    // Read resource
    info!("Reading resource...");
    let resource_result =
        client.read_resource(ReadResourceRequestParams { uri: "file:///bioma/README.md".to_string() }).await?;
    info!("Resource content: {:?}", resource_result.contents);

    // Make an echo tool call
    info!("Making echo tool call...");
    let echo_args = serde_json::json!({
        "message": "Hello from MCP client!"
    });
    let echo_args =
        CallToolRequestParams { name: "echo".to_string(), arguments: serde_json::from_value(echo_args).unwrap() };
    let echo_result = client.call_tool(echo_args).await?;
    info!("Echo response: {:?}", echo_result);

    // Shutdown the client
    info!("Shutting down client...");

    Ok(())
}
