use anyhow::Result;
use bioma_tool::transport::sse::{ClientId, SseClientConfig, SseServerConfig, SseTransport, TransportMessage};
use bioma_tool::transport::Transport;
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
    let content = "Test message content";
    let message = TransportMessage::new(content.to_string());

    // Test as_ref
    assert_eq!(message.as_ref(), content);

    // Test into_inner
    let inner = message.into_inner();
    assert_eq!(inner, content);
}

#[tokio::test]
async fn test_jsonrpc_id_extraction() {
    // Test with string ID
    let message = r#"{"jsonrpc":"2.0","method":"test","params":{},"id":"test-id"}"#;
    let id = SseTransport::extract_jsonrpc_id(message);
    assert_eq!(id, Some("test-id".to_string()));

    // Test with numeric ID
    let message = r#"{"jsonrpc":"2.0","method":"test","params":{},"id":123}"#;
    let id = SseTransport::extract_jsonrpc_id(message);
    assert_eq!(id, Some("123".to_string()));

    // Test with no ID
    let message = r#"{"jsonrpc":"2.0","method":"test","params":{}}"#;
    let id = SseTransport::extract_jsonrpc_id(message);
    assert_eq!(id, None);

    // Test with invalid JSON
    let message = "not json";
    let id = SseTransport::extract_jsonrpc_id(message);
    assert_eq!(id, None);
}

#[tokio::test]
async fn test_sse_event_format_parse() {
    let event_type = "test_event";
    let data = r#"{"key":"value"}"#;

    // Create event string manually to test parse_sse_event
    let event = format!("event: {}\ndata: {}\n\n", event_type, data);

    // Parse it back with our function
    let (parsed_type, parsed_data) = SseTransport::parse_sse_event(&event);

    assert_eq!(parsed_type, Some(event_type.to_string()));
    assert_eq!(parsed_data, Some(data.to_string()));

    // Test with missing type
    let event = format!("data: {}\n\n", data);
    let (parsed_type, parsed_data) = SseTransport::parse_sse_event(&event);

    assert_eq!(parsed_type, None);
    assert_eq!(parsed_data, Some(data.to_string()));
}

// TODO: Not passing rn.
#[tokio::test]
async fn test_sse_basic_functionality() -> Result<()> {
    // Find an available port
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    drop(listener);

    // Create server config and start server
    let server_config = SseServerConfig::builder().url(addr).build();
    let mut server = SseTransport::new_server(server_config);

    // Create server channel
    let (server_tx, mut server_rx) = mpsc::channel::<String>(32);

    // Start server in background
    let server_handle = tokio::spawn(async move {
        if let Err(e) = server.start(server_tx).await {
            eprintln!("Server error: {:?}", e);
        }
    });

    // Wait for server to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Create and start client
    let client_config = SseClientConfig::builder().server_url(addr).build();
    let mut client1 = SseTransport::new_client(client_config)?;

    // Create client channel
    let (client_tx, mut client_rx) = mpsc::channel::<String>(32);

    // Start client in background
    let client_handle = tokio::spawn(async move {
        if let Err(e) = client1.start(client_tx).await {
            eprintln!("Client error: {:?}", e);
        }
    });

    // Client should receive an endpoint event
    let message = timeout(Duration::from_secs(2), client_rx.recv()).await?.unwrap();
    assert!(message.contains("event: endpoint"), "Client should receive endpoint event");
    assert!(message.contains("client_id"), "Endpoint event should contain client_id");

    // Create another client instance for sending messages
    let client_config = SseClientConfig::builder().server_url(addr).build();
    let mut client2 = SseTransport::new_client(client_config)?;

    // Wait for connection establishment
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Send a message from client to server
    let test_request = r#"{"jsonrpc":"2.0","method":"test","params":{},"id":"test-id"}"#;
    client2.send(test_request.to_string()).await?;

    // Server should receive the message
    let received = timeout(Duration::from_secs(2), server_rx.recv()).await?.unwrap();
    assert_eq!(received, test_request);

    // Create a server instance for sending responses
    let server_config = SseServerConfig::builder().url(addr).build();
    let mut server2 = SseTransport::new_server(server_config);

    // Send response from server to client
    let test_response = r#"{"jsonrpc":"2.0","result":{},"id":"test-id"}"#;
    server2.send(test_response.to_string()).await?;

    // Client should receive the response
    let response_event = timeout(Duration::from_secs(2), client_rx.recv()).await?.unwrap();
    assert!(response_event.contains("event: message"), "Event should be of message type");
    assert!(response_event.contains(test_response), "Response should contain the expected JSON");

    // Cleanup
    server_handle.abort();
    client_handle.abort();

    Ok(())
}

#[tokio::test]
async fn test_client_retry() -> Result<()> {
    // Use a non-existent server address
    let addr = "127.0.0.1:49152".parse::<SocketAddr>()?;

    // Configure client with retries
    let client_config =
        SseClientConfig::builder().server_url(addr).retry_count(2).retry_delay(Duration::from_millis(100)).build();

    let mut client = SseTransport::new_client(client_config)?;
    let (tx, _) = mpsc::channel::<String>(1);

    // Start timing
    let start = std::time::Instant::now();

    // Start the client - should fail after retries
    let result = client.start(tx).await;

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

    // Start server
    let server_config = SseServerConfig::builder().url(addr).build();
    let mut server = SseTransport::new_server(server_config);
    let (server_tx, mut server_rx) = mpsc::channel::<String>(32);

    tokio::spawn(async move {
        if let Err(e) = server.start(server_tx).await {
            eprintln!("Server error: {:?}", e);
        }
    });

    // Wait for server to start
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Start multiple clients
    let client_count = 3;
    let mut client_handles = Vec::new();
    let mut client_rxs = Vec::new();

    for i in 0..client_count {
        let client_config = SseClientConfig::builder().server_url(addr).build();
        let mut client = SseTransport::new_client(client_config)?;
        let (tx, rx) = mpsc::channel::<String>(32);

        let handle = tokio::spawn(async move {
            if let Err(e) = client.start(tx).await {
                eprintln!("Client {} error: {:?}", i, e);
            }
        });

        client_handles.push(handle);
        client_rxs.push(rx);
    }

    // Wait for clients to connect and receive their endpoint events
    for rx in &mut client_rxs {
        let _ = timeout(Duration::from_secs(2), rx.recv()).await?;
    }

    // Create a client for sending messages
    let client_config = SseClientConfig::builder().server_url(addr).build();
    let mut sender = SseTransport::new_client(client_config)?;

    // Wait for connection
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Send messages with unique IDs
    for i in 0..client_count {
        let message = format!(r#"{{"jsonrpc":"2.0","method":"test","params":{{}},"id":"client-{}"}}"#, i);
        sender.send(message).await?;

        // Allow server to process
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Verify server received all messages
    for i in 0..client_count {
        if let Ok(Some(received)) = timeout(Duration::from_secs(1), server_rx.recv()).await {
            assert!(received.contains(&format!("client-{}", i)), "Server should receive message from client {}", i);
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
    assert_eq!(default_config.url.to_string(), "127.0.0.1:8080");
    assert_eq!(default_config.channel_capacity, 32);
    assert!(default_config.keep_alive);

    // Test builder pattern
    let custom_addr: SocketAddr = "127.0.0.1:9090".parse().unwrap();
    let custom_config = SseServerConfig::builder().url(custom_addr).channel_capacity(64).keep_alive(false).build();

    assert_eq!(custom_config.url, custom_addr);
    assert_eq!(custom_config.channel_capacity, 64);
    assert!(!custom_config.keep_alive);
}

#[tokio::test]
async fn test_client_config() {
    // Test default config
    let default_config = SseClientConfig::default();
    assert_eq!(default_config.server_url.to_string(), "127.0.0.1:8080");
    assert_eq!(default_config.retry_count, 3);
    assert_eq!(default_config.retry_delay, Duration::from_secs(5));

    // Test builder pattern
    let custom_addr: SocketAddr = "127.0.0.1:9090".parse().unwrap();
    let custom_config =
        SseClientConfig::builder().server_url(custom_addr).retry_count(5).retry_delay(Duration::from_secs(2)).build();

    assert_eq!(custom_config.server_url, custom_addr);
    assert_eq!(custom_config.retry_count, 5);
    assert_eq!(custom_config.retry_delay, Duration::from_secs(2));
}

#[tokio::test]
async fn test_error_handling() -> Result<()> {
    // Test connection to non-existent server with no retries
    let addr = "127.0.0.1:49153".parse::<SocketAddr>()?;
    let client_config = SseClientConfig::builder().server_url(addr).retry_count(0).build();

    let mut client = SseTransport::new_client(client_config)?;
    let (tx, _) = mpsc::channel::<String>(1);

    let result = client.start(tx).await;
    assert!(result.is_err(), "Connection to non-existent server should fail");

    // Test sending a message before connection is established
    let client_config = SseClientConfig::builder().server_url(addr).build();
    let mut client = SseTransport::new_client(client_config)?;

    let result = client.send("test message".to_string()).await;
    assert!(result.is_err(), "Sending without connection should fail");

    Ok(())
}
