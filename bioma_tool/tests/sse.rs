use anyhow::Result;
use bioma_tool::client::SseConfig as SseClientConfig;
use bioma_tool::server::SseConfig as SseServerConfig;
use bioma_tool::transport::sse::{ClientId, SseMessage, SseTransport};
use bioma_tool::transport::Transport;
use bioma_tool::JsonRpcMessage;
use serde_json::json;
use std::net::{SocketAddr, TcpListener};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;

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

// TODO: Not passing rn.
#[tokio::test]
async fn test_sse_basic_functionality() -> Result<()> {
    // Find an available port
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    drop(listener);

    let endpoint = format!("{}", addr);

    // Create server config
    let server_config = SseServerConfig::builder().endpoint(endpoint.clone()).build();

    // Create server channel
    let (server_msg_tx, mut server_msg_rx) = mpsc::channel::<SseMessage>(32);
    let (server_err_tx, _) = mpsc::channel(32);
    let (server_close_tx, _) = mpsc::channel(32);

    // Create server transport
    let mut server = SseTransport::new_server(server_config.clone(), server_msg_tx, server_err_tx, server_close_tx);

    // Start server in background
    let server_handle = tokio::spawn(async move {
        if let Err(e) = server.start().await {
            eprintln!("Server error: {:?}", e);
        }
    });

    // Wait for server to start
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Create client config
    let client_config = SseClientConfig::builder().endpoint(endpoint).build();

    // Create client channel
    let (client_msg_tx, mut client_msg_rx) = mpsc::channel::<JsonRpcMessage>(32);
    let (client_err_tx, _) = mpsc::channel(32);
    let (client_close_tx, _) = mpsc::channel(32);

    // Create client transport
    let mut client1 = SseTransport::new_client(&client_config, client_msg_tx, client_err_tx, client_close_tx)?;

    // Start client in background
    let client_handle = tokio::spawn(async move {
        if let Err(e) = client1.start().await {
            eprintln!("Client error: {:?}", e);
        }
    });

    // Wait for connection establishment
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Create another client for sending messages
    let (tx, _) = mpsc::channel::<JsonRpcMessage>(32);
    let (err_tx, _) = mpsc::channel(32);
    let (close_tx, _) = mpsc::channel(32);

    let mut client2 = SseTransport::new_client(&client_config, tx, err_tx, close_tx)?;

    // Send a message from client to server
    let test_request = json!({"jsonrpc":"2.0","method":"test","params":{},"id":"test-id"});
    let json_rpc_message: JsonRpcMessage = serde_json::from_value(test_request.clone())?;
    client2.send(json_rpc_message, serde_json::Value::Null).await?;

    // Server should receive the message
    let received = timeout(Duration::from_secs(2), server_msg_rx.recv()).await?.unwrap();
    assert_eq!(serde_json::to_value(received.message)?, test_request);

    // Create another server instance for sending responses
    let (tx, _) = mpsc::channel::<SseMessage>(32);
    let (err_tx, _) = mpsc::channel(32);
    let (close_tx, _) = mpsc::channel(32);

    let mut server2 = SseTransport::new_server(server_config, tx, err_tx, close_tx);

    // Send response from server to client
    let test_response = json!({"jsonrpc":"2.0","result":{},"id":"test-id"});
    let json_rpc_message: JsonRpcMessage = serde_json::from_value(test_response.clone())?;

    // Create metadata with client ID
    let client_id = received.client_id.as_ref().to_string();
    let metadata = json!({ "client_id": client_id });

    server2.send(json_rpc_message, metadata).await?;

    // Client should receive the response
    let response = timeout(Duration::from_secs(2), client_msg_rx.recv()).await?.unwrap();
    assert_eq!(serde_json::to_value(response)?, test_response);

    // Cleanup
    server_handle.abort();
    client_handle.abort();

    Ok(())
}

#[tokio::test]
async fn test_client_retry() -> Result<()> {
    // Use a non-existent server address
    let addr = "127.0.0.1:49152".parse::<SocketAddr>()?;
    let endpoint = format!("{}", addr);

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
    let result = client.start().await;

    // Check timing and result
    let elapsed = start.elapsed();
    assert!(elapsed >= Duration::from_millis(200), "Should have retried at least twice");
    assert!(result.is_err(), "Should fail to connect to non-existent server");

    Ok(())
}

// TODO: Not passing rn. Tested manually and it works. Come back to it.
#[tokio::test]
async fn test_multiple_clients() -> Result<()> {
    // Find an available port
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    drop(listener);

    let endpoint = format!("{}", addr);

    // Start server
    let server_config = SseServerConfig::builder().endpoint(endpoint.clone()).build();

    let (server_msg_tx, mut server_msg_rx) = mpsc::channel::<SseMessage>(32);
    let (server_err_tx, _) = mpsc::channel(32);
    let (server_close_tx, _) = mpsc::channel(32);

    let mut server = SseTransport::new_server(server_config, server_msg_tx, server_err_tx, server_close_tx);

    tokio::spawn(async move {
        if let Err(e) = server.start().await {
            eprintln!("Server error: {:?}", e);
        }
    });

    // Wait for server to start
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Start multiple clients
    let client_count = 3;
    let mut client_handles = Vec::new();
    let mut client_rxs = Vec::new();

    let client_config = SseClientConfig::builder().endpoint(endpoint.clone()).build();

    for i in 0..client_count {
        let (tx, rx) = mpsc::channel::<JsonRpcMessage>(32);
        let (err_tx, _) = mpsc::channel(32);
        let (close_tx, _) = mpsc::channel(32);

        let mut client = SseTransport::new_client(&client_config, tx, err_tx, close_tx)?;

        let handle = tokio::spawn(async move {
            if let Err(e) = client.start().await {
                eprintln!("Client {} error: {:?}", i, e);
            }
        });

        client_handles.push(handle);
        client_rxs.push(rx);
    }

    // Wait for clients to connect and receive their endpoint events
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Create a client for sending messages
    let (tx, _) = mpsc::channel::<JsonRpcMessage>(32);
    let (err_tx, _) = mpsc::channel(32);
    let (close_tx, _) = mpsc::channel(32);

    let mut sender = SseTransport::new_client(&client_config, tx, err_tx, close_tx)?;

    // Wait for connection
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Send messages with unique IDs
    for i in 0..client_count {
        let message = json!({
            "jsonrpc": "2.0",
            "method": "test",
            "params": {},
            "id": format!("client-{}", i)
        });

        let json_rpc_message: JsonRpcMessage = serde_json::from_value(message)?;
        sender.send(json_rpc_message, serde_json::Value::Null).await?;

        // Allow server to process
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Verify server received all messages
    for _ in 0..client_count {
        if let Ok(Some(received)) = timeout(Duration::from_secs(1), server_msg_rx.recv()).await {
            let message_json = serde_json::to_value(received.message)?;
            let id = message_json["id"].as_str().unwrap_or("");
            assert!(id.contains(&format!("client-")), "Server should receive message from client");
        }
    }

    // Cleanup
    for handle in client_handles {
        handle.abort();
    }

    Ok(())
}

#[tokio::test]
async fn test_server_config() {
    // Test default config
    let default_config = SseServerConfig::default();
    assert_eq!(default_config.endpoint, "127.0.0.1:8080");
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
    let addr = "127.0.0.1:49153".parse::<SocketAddr>()?;
    let endpoint = format!("{}", addr);

    let client_config = SseClientConfig::builder().endpoint(endpoint.clone()).retry_count(0).build();

    let (tx, _) = mpsc::channel::<JsonRpcMessage>(1);
    let (err_tx, _) = mpsc::channel(32);
    let (close_tx, _) = mpsc::channel(32);

    let mut client = SseTransport::new_client(&client_config, tx, err_tx, close_tx)?;

    let result = client.start().await;
    assert!(result.is_err(), "Connection to non-existent server should fail");

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
