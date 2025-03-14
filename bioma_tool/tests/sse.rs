use anyhow::Result;
use bioma_tool::client::SseConfig as SseClientConfig;
use bioma_tool::server::SseConfig as SseServerConfig;
use bioma_tool::transport::sse::{ClientId, SseTransport};
use bioma_tool::transport::Transport;
use bioma_tool::JsonRpcMessage;
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
    let id_str = id1.as_ref();
    assert!(!id_str.is_empty(), "ID string should not be empty");
    assert_eq!(id_str.len(), 36, "Client ID should be a UUID (36 chars)");

    // Test cloning
    let id1_clone = id1.clone();
    assert_eq!(id1, id1_clone, "Cloned ID should equal original");
}

#[tokio::test]
async fn test_transport_message() {
    // Create a JsonRpcMessage
    let message = json!({"jsonrpc": "2.0", "method": "test", "params": {}, "id": 1});
    let json_rpc_message: JsonRpcMessage = serde_json::from_value(message).unwrap();

    // Create a TransportMessage
    let transport_message = bioma_tool::transport::sse::TransportMessage::new(json_rpc_message.clone());

    // Test properties
    assert_eq!(transport_message.event_type, "message");
    assert_eq!(transport_message.message, json_rpc_message);
}

#[tokio::test]
async fn test_sse_event_format_parse() {
    let event_type = "test_event";
    let data = r#"{"key":"value"}"#;

    // Create event string manually to test parse_sse_event
    let event = format!("event: {}\ndata: {}\n\n", event_type, data);

    // Parse it back with our function
    let parsed_event = SseTransport::parse_sse_event(&event);

    assert_eq!(parsed_event.event_type, Some(event_type.to_string()));
    assert_eq!(parsed_event.data, Some(data.to_string()));

    // Test with missing type
    let event = format!("data: {}\n\n", data);
    let parsed_event = SseTransport::parse_sse_event(&event);

    assert_eq!(parsed_event.event_type, None);
    assert_eq!(parsed_event.data, Some(data.to_string()));
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
    assert_eq!(default_config.endpoint, "127.0.0.1:8090");
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
    assert_eq!(default_config.endpoint, "http://127.0.0.1:8090");
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
