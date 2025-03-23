//! WebSocket transport implementation for JSON-RPC communication.
//!
//! This module provides both client and server implementations for WebSocket-based
//! transport of JSON-RPC messages. The implementation supports:
//!
//! - Server mode: Listens for incoming connections and maintains a registry of connected clients
//! - Client mode: Connects to a WebSocket server and maintains the connection
//!
//! Both modes implement the `Transport` trait, providing a consistent interface for
//! sending and receiving messages.

use super::{SendMessage, Transport, TransportSender};

use crate::client::WsConfig as WsClientConfig;
use crate::server::WsConfig as WsServerConfig;
use crate::transport::Message;
use crate::{ClientId, JsonRpcMessage};

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::{
    accept_async, connect_async,
    tungstenite::protocol::{frame::coding::CloseCode, CloseFrame, Message as WsMessage},
    WebSocketStream,
};
use tracing::{debug, error, info};

/// Errors that can occur during WebSocket transport operations.
#[derive(Debug, Error)]
pub enum WsError {
    /// Error during JSON serialization or deserialization.
    #[error("JSON serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    /// Attempted to send a message to a client that is not registered.
    #[error("Client not found: {0}")]
    ClientNotFound(ClientId),

    /// Failed to send a message through a channel.
    #[error("Message sending failed: {0}")]
    SendError(String),

    /// WebSocket connection error.
    #[error("WebSocket connection error: {0}")]
    ConnectionError(String),

    /// Error binding to network address.
    #[error("Network binding error: {0}")]
    BindError(String),

    /// Error in WebSocket protocol.
    #[error("WebSocket protocol error: {0}")]
    ProtocolError(String),

    /// WebSocket transport not in correct state.
    #[error("Transport state error: {0}")]
    StateError(String),

    /// Invalid message format or content.
    #[error("Message parsing error: {0}")]
    MessageError(String),

    /// Operation timed out.
    #[error("Timeout error: {0}")]
    TimeoutError(String),

    /// WebSocket handshake failed.
    #[error("WebSocket handshake failed: {0}")]
    HandshakeError(String),

    /// Metadata parsing or validation error.
    #[error("Invalid metadata: {0}")]
    MetadataError(String),
}

/// WebSocket stream type used for server connections.
type WsStreamServer = WebSocketStream<TcpStream>;

/// WebSocket stream type used for client connections, may use TLS.
type WsStreamClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Registry of connected clients and their message senders.
type ClientRegistry = Arc<RwLock<HashMap<ClientId, mpsc::Sender<JsonRpcMessage>>>>;

/// Sender half of the WebSocket stream for client mode.
type WsSenderClient = Arc<Mutex<Option<futures_util::stream::SplitSink<WsStreamClient, WsMessage>>>>;

/// Server mode configuration and state.
struct ServerMode {
    /// Registry of connected clients and their message senders.
    clients: ClientRegistry,
    /// WebSocket server endpoint address (e.g., "127.0.0.1:8080").
    endpoint: String,
    /// Channel for forwarding received messages to the application.
    on_message: mpsc::Sender<Message>,
}

/// Client mode configuration and state.
struct ClientMode {
    /// WebSocket server endpoint to connect to (e.g., "ws://127.0.0.1:8080").
    endpoint: String,
    /// Channel for forwarding received messages to the application.
    on_message: mpsc::Sender<JsonRpcMessage>,
    /// Sender half of the WebSocket connection, used to send messages.
    sender: WsSenderClient,
}

/// WebSocket transport operating mode.
enum WsMode {
    /// Server mode with connected clients registry.
    Server(ServerMode),
    /// Client mode connecting to a server.
    Client(ClientMode),
}

/// WebSocket transport implementation that can operate in either server or client mode.
///
/// This struct implements the `Transport` trait and provides WebSocket-based communication
/// for JSON-RPC messages. In server mode, it accepts multiple client connections and routes
/// messages between them. In client mode, it connects to a WebSocket server and exchanges
/// messages with it.
#[derive(Clone)]
pub struct WsTransport {
    /// The operation mode of this transport (server or client).
    mode: Arc<WsMode>,
    /// Channel for reporting transport errors to the application.
    #[allow(unused)]
    on_error: mpsc::Sender<anyhow::Error>,
    /// Channel for notifying the application when the transport is closed.
    on_close: mpsc::Sender<()>,
}

// A separate sender for WsTransport
#[derive(Clone)]
pub struct WsTransportSender {
    mode: Arc<WsMode>,
}

impl SendMessage for WsTransportSender {
    async fn send(&self, message: JsonRpcMessage, client_id: ClientId) -> Result<()> {
        match &*self.mode {
            WsMode::Server(server) => WsTransport::send_to_client(&server.clients, &client_id, message)
                .await
                .map_err(|e| anyhow::Error::msg(format!("Failed to send to client: {}", e))),
            WsMode::Client(client) => {
                // Serialize message to JSON
                let json = serde_json::to_string(&message)
                    .map_err(|e| anyhow::Error::msg(format!("Serialization error: {}", e)))?;

                // Send via WebSocket
                let mut sender_guard = client.sender.lock().await;
                if let Some(sender) = sender_guard.as_mut() {
                    sender
                        .send(WsMessage::Text(json.into()))
                        .await
                        .map_err(|e| anyhow::Error::msg(format!("WebSocket send error: {}", e)))?;
                } else {
                    return Err(anyhow::Error::msg("WebSocket sender not initialized"));
                }

                Ok(())
            }
        }
    }
}

impl WsTransport {
    /// Creates a new WebSocket transport in server mode.
    ///
    /// # Arguments
    ///
    /// * `config` - Server configuration containing the endpoint to listen on.
    /// * `on_message` - Channel for forwarding received messages to the application.
    /// * `on_error` - Channel for reporting transport errors to the application.
    /// * `on_close` - Channel for notifying the application when the transport is closed.
    ///
    /// # Returns
    ///
    /// A new WebSocket transport configured for server mode.
    pub fn new_server(
        config: WsServerConfig,
        on_message: mpsc::Sender<Message>,
        on_error: mpsc::Sender<anyhow::Error>,
        on_close: mpsc::Sender<()>,
    ) -> Self {
        let server_mode =
            ServerMode { clients: Arc::new(RwLock::new(HashMap::new())), endpoint: config.endpoint, on_message };

        Self { mode: Arc::new(WsMode::Server(server_mode)), on_error, on_close }
    }

    /// Creates a new WebSocket transport in client mode.
    ///
    /// # Arguments
    ///
    /// * `config` - Client configuration containing the server endpoint to connect to.
    /// * `on_message` - Channel for forwarding received messages to the application.
    /// * `on_error` - Channel for reporting transport errors to the application.
    /// * `on_close` - Channel for notifying the application when the transport is closed.
    ///
    /// # Returns
    ///
    /// A new WebSocket transport configured for client mode, or an error if configuration is invalid.
    pub fn new_client(
        config: &WsClientConfig,
        on_message: mpsc::Sender<JsonRpcMessage>,
        on_error: mpsc::Sender<anyhow::Error>,
        on_close: mpsc::Sender<()>,
    ) -> Result<Self> {
        let client_mode =
            ClientMode { endpoint: config.endpoint.clone(), on_message, sender: Arc::new(Mutex::new(None)) };

        Ok(Self { mode: Arc::new(WsMode::Client(client_mode)), on_error, on_close })
    }

    /// Handles a single client WebSocket connection in server mode.
    ///
    /// This function manages the lifecycle of a client connection, including:
    /// - Splitting the WebSocket stream into sender and receiver parts
    /// - Registering the client in the client registry
    /// - Processing incoming messages from the client
    /// - Forwarding messages to the client
    /// - Cleaning up when the connection closes
    ///
    /// # Arguments
    ///
    /// * `ws_stream` - The WebSocket stream for this client connection.
    /// * `client_id` - Unique identifier for this client.
    /// * `clients` - Registry of connected clients to register this client in.
    /// * `on_message` - Channel for forwarding received messages to the application.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the connection was handled successfully, or an error if something went wrong.
    async fn handle_client_connection(
        ws_stream: WsStreamServer,
        client_id: ClientId,
        clients: ClientRegistry,
        on_message: mpsc::Sender<Message>,
    ) -> Result<(), WsError> {
        // Split the WebSocket stream into sender and receiver parts
        let (mut ws_sender, mut ws_receiver) = ws_stream.split();

        // Create a channel for this client
        let (client_sender, mut client_receiver) = mpsc::channel(32);

        // Register the client in the client registry
        {
            let mut clients = clients.write().await;
            clients.insert(client_id.clone(), client_sender);
        }

        info!("WebSocket client connected: {}", client_id);

        // Task for forwarding messages from the client channel to the WebSocket
        let client_task = tokio::spawn(async move {
            while let Some(message) = client_receiver.recv().await {
                let json = serde_json::to_string(&message)?;
                if ws_sender.send(WsMessage::Text(json.into())).await.is_err() {
                    // Connection was closed, exit the loop
                    break;
                }
            }
            Ok::<_, WsError>(())
        });

        // Process incoming messages from the WebSocket
        while let Some(result) = ws_receiver.next().await {
            match result {
                Ok(WsMessage::Text(text)) => {
                    debug!("Received WebSocket message: {}", text);
                    match serde_json::from_str::<JsonRpcMessage>(&text) {
                        Ok(message) => {
                            let ws_message = Message { client_id: client_id.clone(), message };
                            if on_message.send(ws_message).await.is_err() {
                                // Channel was closed, exit the loop
                                break;
                            }
                        }
                        Err(err) => {
                            let error = WsError::MessageError(format!("Failed to parse JSON-RPC message: {}", err));
                            error!("{}", error);
                            // Continue processing other messages despite this error
                        }
                    }
                }
                Ok(WsMessage::Close(_)) => break, // Client initiated close
                Err(err) => {
                    error!("WebSocket connection error: {}", err);
                    break;
                }
                _ => {} // Ignore other message types
            }
        }

        // Clean up client registration when connection closes
        {
            let mut clients = clients.write().await;
            clients.remove(&client_id);
        }

        info!("WebSocket client disconnected: {}", client_id);
        client_task.abort(); // Cancel the client forwarding task
        Ok(())
    }

    /// Sends a message to a specific client in server mode.
    ///
    /// # Arguments
    ///
    /// * `clients` - Registry of connected clients.
    /// * `client_id` - The client to send the message to.
    /// * `message` - The JSON-RPC message to send.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the message was sent successfully, or an error if the client was not found
    /// or the message could not be sent.
    ///
    /// # Errors
    ///
    /// Returns `WsError::ClientNotFound` if the client is not registered.
    /// Returns `WsError::SendError` if the message channel is closed.
    async fn send_to_client(
        clients: &ClientRegistry,
        client_id: &ClientId,
        message: JsonRpcMessage,
    ) -> Result<(), WsError> {
        let clients = clients.read().await;
        if let Some(sender) = clients.get(client_id) {
            sender
                .send(message)
                .await
                .map_err(|_| WsError::SendError(format!("Failed to send message to client {}", client_id)))?;
            Ok(())
        } else {
            Err(WsError::ClientNotFound(client_id.clone()))
        }
    }
}

impl Transport for WsTransport {
    /// Starts the WebSocket transport.
    ///
    /// In server mode, this starts listening for incoming connections.
    /// In client mode, this connects to the server endpoint.
    ///
    /// # Returns
    ///
    /// A `JoinHandle` that completes when the transport stops, or an error if
    /// the transport could not be started.
    async fn start(&mut self) -> Result<JoinHandle<Result<()>>> {
        let mode = self.mode.clone();

        // For client mode, we'll create a notification channel to wait until sender is set
        let (sender_ready_tx, sender_ready_rx) = tokio::sync::oneshot::channel();
        let sender_ready_tx = Arc::new(Mutex::new(Some(sender_ready_tx)));

        let handle = tokio::spawn(async move {
            match &*mode {
                WsMode::Server(server) => {
                    let clients_clone = server.clients.clone();
                    let on_message_clone = server.on_message.clone();
                    let endpoint_clone = server.endpoint.clone();

                    let server_handle = tokio::spawn(async move {
                        // Bind to the endpoint and start listening for connections
                        let listener = TcpListener::bind(&endpoint_clone)
                            .await
                            .map_err(|e| WsError::BindError(format!("Failed to bind to {}: {}", endpoint_clone, e)))?;

                        info!("WebSocket server listening on {}", endpoint_clone);

                        // Accept and handle incoming connections
                        while let Ok((stream, addr)) = listener.accept().await {
                            let client_id = ClientId::new();
                            debug!("Accepting connection from {} with ID {}", addr, client_id);

                            // Upgrade the TCP stream to a WebSocket stream
                            let ws_stream = accept_async(stream).await.map_err(|e| {
                                WsError::HandshakeError(format!("Failed to accept WebSocket connection: {}", e))
                            })?;

                            let clients = clients_clone.clone();
                            let on_message = on_message_clone.clone();
                            let client_id_clone = client_id.clone();

                            // Spawn a task to handle this client connection
                            tokio::spawn(async move {
                                if let Err(e) = WsTransport::handle_client_connection(
                                    ws_stream,
                                    client_id_clone,
                                    clients,
                                    on_message,
                                )
                                .await
                                {
                                    error!("Error handling WebSocket connection: {}", e);
                                }
                            });
                        }

                        Ok(())
                    });

                    return server_handle.await?;
                }
                WsMode::Client(client) => {
                    let endpoint_clone = client.endpoint.clone();
                    let on_message_clone = client.on_message.clone();
                    let sender_clone = client.sender.clone();
                    let sender_ready = sender_ready_tx.clone();

                    info!("Connecting to WebSocket server at {}", endpoint_clone);

                    // Connect to the server
                    let (ws_stream, _) = connect_async(&endpoint_clone).await.map_err(|e| {
                        WsError::ConnectionError(format!("Failed to connect to {}: {}", endpoint_clone, e))
                    })?;

                    // Split the WebSocket stream into sender and receiver parts
                    let (ws_sender, mut ws_receiver) = ws_stream.split();

                    // Store the sender in the client mode for later use
                    {
                        let mut client_sender = sender_clone.lock().await;
                        *client_sender = Some(ws_sender);

                        // Signal that the sender is ready
                        if let Some(tx) = sender_ready.lock().await.take() {
                            let _ = tx.send(());
                        }
                    }

                    // Process incoming messages in a separate task
                    let receive_task = tokio::spawn(async move {
                        while let Some(result) = ws_receiver.next().await {
                            match result {
                                Ok(WsMessage::Text(text)) => {
                                    debug!("Client received [ws]: {}", text);
                                    match serde_json::from_str::<JsonRpcMessage>(&text) {
                                        Ok(message) => {
                                            if on_message_clone.send(message).await.is_err() {
                                                // Channel was closed, exit the loop
                                                break;
                                            }
                                        }
                                        Err(err) => {
                                            let error = WsError::MessageError(format!(
                                                "Failed to parse JSON-RPC message: {}",
                                                err
                                            ));
                                            error!("{}", error);
                                            // Continue processing other messages despite this error
                                        }
                                    }
                                }
                                Ok(WsMessage::Close(_)) => break, // Server initiated close
                                Err(err) => {
                                    error!("WebSocket connection error: {}", err);
                                    break;
                                }
                                _ => {} // Ignore other message types
                            }
                        }

                        Ok(())
                    });

                    return receive_task.await?;
                }
            }
        });

        // For client mode, wait for sender to be ready before returning the handle
        if let WsMode::Client(_) = &*self.mode {
            // Wait with a reasonable timeout
            match tokio::time::timeout(std::time::Duration::from_secs(10), sender_ready_rx).await {
                Ok(Ok(())) => {
                    debug!("WebSocket client sender is ready");
                }
                Ok(Err(_)) => {
                    return Err(WsError::StateError("Failed to initialize client sender".to_string()).into());
                }
                Err(_) => {
                    return Err(
                        WsError::TimeoutError("Timeout waiting for client sender to be ready".to_string()).into()
                    );
                }
            }
        }

        Ok(handle)
    }

    /// Sends a JSON-RPC message through the WebSocket transport.
    ///
    /// In server mode, the message is sent to a specific client identified by the metadata.
    /// In client mode, the message is sent to the server.
    ///
    /// # Arguments
    ///
    /// * `message` - The JSON-RPC message to send.
    /// * `client_id` - In server mode, this must contain a client ID. In client mode, this is ignored.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the message was sent successfully, or an error if the message could not be sent.
    ///
    /// # Errors
    ///
    /// Returns an error if the transport is not started, the client is not found, or the message
    /// could not be serialized or sent.
    async fn send(&mut self, message: JsonRpcMessage, client_id: ClientId) -> Result<()> {
        match &*self.mode {
            WsMode::Server(server) => {
                debug!("Server sending WebSocket message");
                Self::send_to_client(&server.clients, &client_id, message).await.map_err(|e| anyhow::anyhow!("{}", e))
            }
            WsMode::Client(client) => {
                debug!("Client sending WebSocket message");
                let mut sender_guard = client.sender.lock().await;
                match &mut *sender_guard {
                    Some(sender) => {
                        let message_json = serde_json::to_string(&message)?;
                        sender.send(WsMessage::Text(message_json.into())).await?;
                        Ok(())
                    }
                    None => Err(anyhow::anyhow!("WebSocket connection not established")),
                }
            }
        }
    }

    /// Closes the WebSocket transport.
    ///
    /// In server mode, this sends WebSocket close frames to all clients
    /// and clears the client registry.
    /// In client mode, this sends a WebSocket close frame to the server
    /// and waits for the connection to close gracefully.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the transport was closed successfully, or an error if the transport
    /// could not be closed.
    async fn close(&mut self) -> Result<()> {
        match &*self.mode {
            WsMode::Server(server) => {
                // Get all client senders
                let mut clients = server.clients.write().await;
                let client_count = clients.len();

                if client_count > 0 {
                    info!("Initiating shutdown for {} client connections", client_count);

                    // To send close frames, we need to access the actual WebSocket streams
                    // For server mode, we don't have direct access to those streams from here
                    // Instead, we'll rely on dropping all senders which will cause the client
                    // handling tasks to exit

                    // Clear the client registry to drop all senders
                    clients.clear();
                    debug!("Cleared client registry, connection handlers will clean up");
                } else {
                    debug!("No clients to close");
                }

                info!("All client connections marked for closure");

                // Signal that the transport is closed
                if let Err(e) = self.on_close.send(()).await {
                    error!("Failed to send close notification: {}", e);
                }

                Ok(())
            }
            WsMode::Client(client) => {
                // In client mode, send a proper close frame to the server
                let mut sender_guard = client.sender.lock().await;

                if let Some(ref mut sender) = *sender_guard {
                    info!("Initiating graceful shutdown of WebSocket connection");

                    // Create a close frame with normal close code (1000)
                    let close_frame = WsMessage::Close(Some(CloseFrame {
                        code: CloseCode::Normal,
                        reason: "Client initiated shutdown".into(),
                    }));

                    // Send the close frame
                    match sender.send(close_frame).await {
                        Ok(_) => {
                            debug!("WebSocket close frame sent successfully");

                            // Wait for the connection to close gracefully
                            debug!("Waiting for connection to close gracefully");
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        }
                        Err(e) => {
                            error!("Error sending WebSocket close frame: {}", e);
                        }
                    }

                    // Set the sender to None to prevent further usage
                    *sender_guard = None;
                } else {
                    debug!("WebSocket connection already closed");
                }

                // Signal that the transport is closed
                if let Err(e) = self.on_close.send(()).await {
                    error!("Failed to send close notification: {}", e);
                }

                Ok(())
            }
        }
    }

    fn sender(&self) -> TransportSender {
        TransportSender::new_ws(WsTransportSender { mode: self.mode.clone() })
    }
}
