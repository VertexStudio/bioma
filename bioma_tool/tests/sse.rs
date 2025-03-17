use anyhow::Result;
use bioma_tool::client::SseConfig as SseClientConfig;
use bioma_tool::server::SseConfig as SseServerConfig;
use bioma_tool::transport::sse::{SseEvent, SseTransport};
use bioma_tool::transport::Transport;
use bioma_tool::{ClientId, JsonRpcMessage};
use serde_json::json;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::mpsc;

// Basic functionality tests
#[tokio::test]
async fn test_client_id() {
    // Test ID generation uniqueness
    let id1 = ClientId::new();
    let id2 = ClientId::new();
    assert_ne!(id1, id2, "Generated IDs should be unique");

    // Test string conversion
    let id_str = id1.to_string();
    assert!(!id_str.is_empty(), "ID string should not be empty");
    assert_eq!(id_str.len(), 36, "Client ID should be a UUID (36 chars)");

    // Test cloning
    let id1_clone = id1.clone();
    assert_eq!(id1, id1_clone, "Cloned ID should equal original");
}

#[tokio::test]
async fn test_sse_event() {
    // Create a JsonRpcMessage
    let message = json!({"jsonrpc": "2.0", "method": "test", "params": {}, "id": 1});
    let json_rpc_message: JsonRpcMessage = serde_json::from_value(message).unwrap();

    // Create an SSE event
    let event = SseEvent::Message(json_rpc_message.clone());

    // Test event type
    let event_str = event.to_sse_string().unwrap();
    assert!(event_str.contains("event: message"), "Event type should be 'message'");

    // Test endpoint event
    let endpoint = "http://example.com";
    let endpoint_event = SseEvent::Endpoint(endpoint.to_string());
    let endpoint_str = endpoint_event.to_sse_string().unwrap();
    assert!(endpoint_str.contains("event: endpoint"), "Event type should be 'endpoint'");
    assert!(endpoint_str.contains(endpoint), "Event should contain the endpoint URL");

    // Test shutdown event
    let shutdown_event =
        SseEvent::Shutdown(bioma_tool::transport::sse::Shutdown { reason: "test shutdown".to_string() });
    let shutdown_str = shutdown_event.to_sse_string().unwrap();
    assert!(shutdown_str.contains("event: shutdown"), "Event type should be 'shutdown'");
    assert!(shutdown_str.contains("test shutdown"), "Event should contain the shutdown reason");
}

#[tokio::test]
async fn test_sse_event_parsing() {
    // Test message event parsing
    let message = json!({"jsonrpc": "2.0", "method": "test", "params": {}, "id": 1});
    let event_str = format!("event: message\ndata: {}\n\n", message);
    let parsed = SseEvent::from_sse_string(&event_str).unwrap().unwrap();
    assert!(matches!(parsed, SseEvent::Message(_)), "Should parse as message event");

    // Test endpoint event parsing
    let endpoint = "http://example.com";
    let event_str = format!("event: endpoint\ndata: \"{}\"\n\n", endpoint);
    let parsed = SseEvent::from_sse_string(&event_str).unwrap().unwrap();
    assert!(matches!(parsed, SseEvent::Endpoint(_)), "Should parse as endpoint event");

    // Test shutdown event parsing
    let shutdown = json!({"reason": "test shutdown"});
    let event_str = format!("event: shutdown\ndata: {}\n\n", shutdown);
    let parsed = SseEvent::from_sse_string(&event_str).unwrap().unwrap();
    assert!(matches!(parsed, SseEvent::Shutdown(_)), "Should parse as shutdown event");

    // Test invalid event parsing
    let invalid_event = "invalid event format\n\n";
    let parsed = SseEvent::from_sse_string(invalid_event).unwrap();
    assert!(parsed.is_none(), "Should return None for invalid event");
}

#[tokio::test]
async fn test_client_retry() -> Result<()> {
    // Use a non-existent server address
    let endpoint = "http://127.0.0.1:49153".to_string();

    // Configure client with retries
    let client_config =
        SseClientConfig::builder().endpoint(endpoint).retry_count(2).retry_delay(Duration::from_millis(100)).build();

    // Create channels
    let (tx, _) = mpsc::channel::<JsonRpcMessage>(1);
    let (err_tx, _) = mpsc::channel(32);
    let (close_tx, _) = mpsc::channel(32);

    let mut client = SseTransport::new_client(&client_config, tx, err_tx, close_tx)?;

    // Start timing
    let start = std::time::Instant::now();

    // Start the client - should fail after retries
    let handle = client.start().await?;
    let result = handle.await;

    // Check timing and result
    let elapsed = start.elapsed();
    assert!(elapsed >= Duration::from_millis(200), "Should have retried at least twice");
    assert!(result.is_err() || result.unwrap().is_err(), "Should fail to connect to non-existent server");

    Ok(())
}

#[tokio::test]
async fn test_server_config() {
    // Test default config
    let default_config = SseServerConfig::default();
    assert_eq!(default_config.endpoint, "127.0.0.1:8090/sse");
    assert_eq!(default_config.channel_capacity, 32);
    assert!(default_config.keep_alive);

    // Test builder pattern
    let custom_addr: SocketAddr = "127.0.0.1:9090".parse().unwrap();
    let custom_config =
        SseServerConfig::builder().endpoint(custom_addr.to_string()).channel_capacity(64).keep_alive(false).build();

    assert_eq!(custom_config.endpoint, "127.0.0.1:9090");
    assert_eq!(custom_config.channel_capacity, 64);
    assert!(!custom_config.keep_alive);
}

#[tokio::test]
async fn test_client_config() {
    // Test default config
    let default_config = SseClientConfig::default();
    assert_eq!(default_config.endpoint, "http://127.0.0.1:8090/sse");
    assert_eq!(default_config.retry_count, 3);
    assert_eq!(default_config.retry_delay, Duration::from_secs(5));

    // Test builder pattern
    let custom_addr: SocketAddr = "127.0.0.1:9090".parse().unwrap();
    let custom_config = SseClientConfig::builder()
        .endpoint(custom_addr.to_string())
        .retry_count(5)
        .retry_delay(Duration::from_secs(2))
        .build();

    assert_eq!(custom_config.endpoint, "127.0.0.1:9090");
    assert_eq!(custom_config.retry_count, 5);
    assert_eq!(custom_config.retry_delay, Duration::from_secs(2));
}

#[tokio::test]
async fn test_error_handling() -> Result<()> {
    // Test connection to non-existent server with no retries
    let endpoint = "http://127.0.0.1:49153".to_string();

    let client_config = SseClientConfig::builder().endpoint(endpoint.clone()).retry_count(0).build();

    let (tx, _) = mpsc::channel::<JsonRpcMessage>(1);
    let (err_tx, _) = mpsc::channel(32);
    let (close_tx, _) = mpsc::channel(32);

    let mut client = SseTransport::new_client(&client_config, tx, err_tx, close_tx)?;

    let handle = client.start().await?;
    let result = handle.await;
    assert!(result.is_err() || result.unwrap().is_err(), "Connection to non-existent server should fail");

    // Test sending a message before connection is established
    let client_config = SseClientConfig::builder().endpoint(endpoint).build();

    let (tx, _) = mpsc::channel::<JsonRpcMessage>(1);
    let (err_tx, _) = mpsc::channel(32);
    let (close_tx, _) = mpsc::channel(32);

    let mut client = SseTransport::new_client(&client_config, tx, err_tx, close_tx)?;

    let test_message = json!({"jsonrpc":"2.0","method":"test","params":{},"id":"test"});
    let json_rpc_message: JsonRpcMessage = serde_json::from_value(test_message)?;

    let result = client.send(json_rpc_message, serde_json::Value::Null).await;
    assert!(result.is_err(), "Sending without connection should fail");

    Ok(())
}
