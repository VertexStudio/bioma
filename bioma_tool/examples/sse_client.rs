use bioma_tool::client::{ModelContextProtocolClient, ServerConfig};
use bioma_tool::schema::Implementation;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize the logger
    let subscriber = FmtSubscriber::builder().with_max_level(Level::DEBUG).finish();
    tracing::subscriber::set_global_default(subscriber).expect("Failed to set global default subscriber");

    // Configure the client to connect to our SSE server
    // The URL is the base HTTP address where the server is running
    // As per MCP spec, the SSE endpoint will be at /events and POST endpoint at /message
    let server_config = ServerConfig {
        name: "example-server".to_string(),
        version: "0.1.0".to_string(),
        transport: "sse".to_string(),
        command: "cargo".to_string(),
        args: vec!["--url=http://127.0.0.1:12345".to_string()],
        request_timeout: 120, // 2 minutes timeout
        enabled: true,
    };

    // Create a client
    let mut client = ModelContextProtocolClient::new(server_config).await?;

    // Initialize the client with proper MCP implementation info
    let client_info = Implementation { name: "sse-test-client".to_string(), version: "0.1.0".to_string() };

    info!("Initializing MCP connection...");
    let mut retries = 0;
    let max_retries = 5;
    let mut init_result = None;

    while retries < max_retries {
        match client.initialize(client_info.clone()).await {
            Ok(result) => {
                init_result = Some(result);
                break;
            }
            Err(e) => {
                error!("Failed to initialize connection, retrying ({}/{}): {}", retries + 1, max_retries, e);
                retries += 1;
                sleep(Duration::from_secs(3)).await;
            }
        }
    }

    let init_result = match init_result {
        Some(result) => result,
        None => {
            error!("Failed to initialize connection after {} retries", max_retries);
            return Err(anyhow::anyhow!("Failed to initialize connection"));
        }
    };

    info!("Server capabilities: {:?}", init_result.capabilities);
    info!("MCP connection established successfully!");

    // Per MCP spec, send initialized notification after initialize request
    info!("Sending initialized notification...");
    client.initialized().await?;
    info!("Initialized notification sent successfully");

    // Test the connection with a ping request (standard MCP method)
    info!("Testing connection with ping request...");
    let mut ping_response = None;
    retries = 0;

    while retries < max_retries {
        match client.request("ping".to_string(), serde_json::json!({})).await {
            Ok(response) => {
                ping_response = Some(response);
                break;
            }
            Err(e) => {
                error!("Ping request failed, retrying ({}/{}): {}", retries + 1, max_retries, e);
                retries += 1;
                sleep(Duration::from_secs(3)).await;
            }
        }
    }

    match ping_response {
        Some(response) => info!("Ping response: {:?}", response),
        None => error!("Ping request failed after {} retries", max_retries),
    }

    // Keep the client running until user terminates it
    info!("MCP client is running successfully. Press Ctrl+C to exit.");
    tokio::signal::ctrl_c().await?;

    info!("Shutting down MCP client");
    Ok(())
}
