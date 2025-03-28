use anyhow::Result;
use bioma_mcp::client::SseConfig as SseClientConfig;
use bioma_mcp::server::SseConfig as SseServerConfig;
use bioma_mcp::transport::sse::{SseEvent, SseTransport};
use bioma_mcp::transport::Transport;
use bioma_mcp::{ConnectionId, JsonRpcMessage};
use serde_json::json;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_client_id() {
    let id1 = ConnectionId::new(None);
    let id2 = ConnectionId::new(None);
    assert_ne!(id1, id2, "Generated IDs should be unique");

    let id_str = id1.to_string();
    assert!(!id_str.is_empty(), "ID string should not be empty");
    assert_eq!(id_str.len(), 36, "Client ID should be a UUID (36 chars)");

    let id1_clone = id1.clone();
    assert_eq!(id1, id1_clone, "Cloned ID should equal original");
}

#[tokio::test]
async fn test_sse_event() {
    let message = json!({"jsonrpc": "2.0", "method": "test", "params": {}, "id": 1});
    let json_rpc_message: JsonRpcMessage = serde_json::from_value(message).unwrap();

    let event = SseEvent::Message(json_rpc_message.clone());

    let event_str = event.to_sse_string().unwrap();
    assert!(event_str.contains("event: message"), "Event type should be 'message'");

    let endpoint = "http://example.com";
    let endpoint_event = SseEvent::Endpoint(endpoint.to_string());
    let endpoint_str = endpoint_event.to_sse_string().unwrap();
    assert!(endpoint_str.contains("event: endpoint"), "Event type should be 'endpoint'");
    assert!(endpoint_str.contains(endpoint), "Event should contain the endpoint URL");

    let shutdown_event =
        SseEvent::Shutdown(bioma_mcp::transport::sse::Shutdown { reason: "test shutdown".to_string() });
    let shutdown_str = shutdown_event.to_sse_string().unwrap();
    assert!(shutdown_str.contains("event: shutdown"), "Event type should be 'shutdown'");
    assert!(shutdown_str.contains("test shutdown"), "Event should contain the shutdown reason");
}

#[tokio::test]
async fn test_sse_event_parsing() {
    let message = json!({"jsonrpc": "2.0", "method": "test", "params": {}, "id": 1});
    let event_str = format!("event: message\ndata: {}\n\n", message);
    let parsed = SseEvent::from_sse_string(&event_str).unwrap().unwrap();
    assert!(matches!(parsed, SseEvent::Message(_)), "Should parse as message event");

    let endpoint = "http://example.com";
    let event_str = format!("event: endpoint\ndata: \"{}\"\n\n", endpoint);
    let parsed = SseEvent::from_sse_string(&event_str).unwrap().unwrap();
    assert!(matches!(parsed, SseEvent::Endpoint(_)), "Should parse as endpoint event");

    let shutdown = json!({"reason": "test shutdown"});
    let event_str = format!("event: shutdown\ndata: {}\n\n", shutdown);
    let parsed = SseEvent::from_sse_string(&event_str).unwrap().unwrap();
    assert!(matches!(parsed, SseEvent::Shutdown(_)), "Should parse as shutdown event");

    let invalid_event = "invalid event format\n\n";
    let parsed = SseEvent::from_sse_string(invalid_event).unwrap();
    assert!(parsed.is_none(), "Should return None for invalid event");
}

#[tokio::test]
async fn test_client_retry() -> Result<()> {
    let endpoint = "http://127.0.0.1:49153".to_string();

    let client_config = SseClientConfig::builder().endpoint(endpoint).build();

    let (tx, _) = mpsc::channel::<JsonRpcMessage>(1);
    let (err_tx, _) = mpsc::channel(32);
    let (close_tx, _) = mpsc::channel(32);

    let mut client = SseTransport::new_client(&client_config, tx, err_tx, close_tx)?;

    let start = std::time::Instant::now();

    let handle = client.start().await?;
    let result = handle.await;

    let elapsed = start.elapsed();
    assert!(elapsed >= Duration::from_millis(200), "Should have retried at least twice");
    assert!(result.is_err() || result.unwrap().is_err(), "Should fail to connect to non-existent server");

    Ok(())
}

#[tokio::test]
async fn test_server_config() {
    let default_config = SseServerConfig::default();
    assert_eq!(default_config.endpoint, "127.0.0.1:8090");
    assert_eq!(default_config.channel_capacity, 32);

    let custom_addr: SocketAddr = "127.0.0.1:9090".parse().unwrap();
    let custom_config = SseServerConfig::builder().endpoint(custom_addr.to_string()).channel_capacity(64).build();

    assert_eq!(custom_config.endpoint, "127.0.0.1:9090");
    assert_eq!(custom_config.channel_capacity, 64);
}

#[tokio::test]
async fn test_client_config() {
    let default_config = SseClientConfig::default();
    assert_eq!(default_config.endpoint, "http://127.0.0.1:8090");

    let custom_addr: SocketAddr = "127.0.0.1:9090".parse().unwrap();
    let custom_config = SseClientConfig::builder().endpoint(custom_addr.to_string()).build();

    assert_eq!(custom_config.endpoint, "127.0.0.1:9090");
}

#[tokio::test]
async fn test_error_handling() -> Result<()> {
    let endpoint = "http://127.0.0.1:49153".to_string();

    let client_config = SseClientConfig::builder().endpoint(endpoint.clone()).build();

    let (tx, _) = mpsc::channel::<JsonRpcMessage>(1);
    let (err_tx, _) = mpsc::channel(32);
    let (close_tx, _) = mpsc::channel(32);

    let mut client = SseTransport::new_client(&client_config, tx, err_tx, close_tx)?;

    let handle = client.start().await?;
    let result = handle.await;
    assert!(result.is_err() || result.unwrap().is_err(), "Connection to non-existent server should fail");

    let client_config = SseClientConfig::builder().endpoint(endpoint).build();

    let (tx, _) = mpsc::channel::<JsonRpcMessage>(1);
    let (err_tx, _) = mpsc::channel(32);
    let (close_tx, _) = mpsc::channel(32);

    let mut client = SseTransport::new_client(&client_config, tx, err_tx, close_tx)?;

    let test_message = json!({"jsonrpc":"2.0","method":"test","params":{},"id":"test"});
    let json_rpc_message: JsonRpcMessage = serde_json::from_value(test_message)?;

    let result = client.send(json_rpc_message, ConnectionId::new(None)).await;
    assert!(result.is_err(), "Sending without connection should fail");

    Ok(())
}
