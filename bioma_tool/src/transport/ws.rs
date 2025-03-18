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

use crate::client::WsConfig as WsClientConfig;
use crate::server::WsConfig as WsServerConfig;
use crate::{ClientId, JsonRpcMessage};

use super::Transport;
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::{accept_async, connect_async, tungstenite::protocol::Message, WebSocketStream};
use tracing::{debug, error, info};

/// Errors that can occur during WebSocket transport operations.
#[derive(Debug, Error)]
enum WsError {
    /// Error during WebSocket connection establishment or communication.
    #[error("WebSocket connection error: {0}")]
    ConnectionError(#[from] tokio_tungstenite::tungstenite::Error),

    /// Error during JSON serialization or deserialization.
    #[error("JSON serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    /// Attempted to send a message to a client that is not registered.
    #[error("Client not found: {0}")]
    ClientNotFound(ClientId),

    /// Failed to send a message through a channel.
    #[error("Message sending failed: {0}")]
    SendError(String),

    /// I/O error during WebSocket operations.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Metadata for WebSocket transport containing client identification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsMetadata {
    /// Unique identifier for the client.
    pub client_id: ClientId,
}

/// WebSocket message with client identification for server-side processing.
#[derive(Debug, Clone)]
pub struct WsMessage {
    /// The JSON-RPC message payload.
    pub message: JsonRpcMessage,
    /// The unique identifier of the client that sent or should receive this message.
    pub client_id: ClientId,
}

/// WebSocket stream type used for server connections.
type WsStreamServer = WebSocketStream<TcpStream>;

/// WebSocket stream type used for client connections, may use TLS.
type WsStreamClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Registry of connected clients and their message senders.
type ClientRegistry = Arc<Mutex<HashMap<ClientId, mpsc::Sender<JsonRpcMessage>>>>;

/// Sender half of the WebSocket stream for client mode.
type WsSenderClient = Arc<Mutex<Option<futures_util::stream::SplitSink<WsStreamClient, Message>>>>;

/// Server mode configuration and state.
struct ServerMode {
    /// Registry of connected clients and their message senders.
    clients: ClientRegistry,
    /// WebSocket server endpoint address (e.g., "127.0.0.1:8080").
    endpoint: String,
    /// Channel for forwarding received messages to the application.
    on_message: mpsc::Sender<WsMessage>,
}

/// Client mode configuration and state.
struct ClientMode {
    /// WebSocket server endpoint to connect to (e.g., "ws://127.0.0.1:8080").
    endpoint: String,
    /// Unique identifier for this client.
    client_id: ClientId,
    /// Channel for forwarding received messages to the application.
    on_message: mpsc::Sender<JsonRpcMessage>,
    /// Sender half of the WebSocket connection, used to send messages.
    sender: WsSenderClient,
}

/// WebSocket transport operation mode.
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
    #[allow(unused)]
    on_close: mpsc::Sender<()>,
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
        on_message: mpsc::Sender<WsMessage>,
        on_error: mpsc::Sender<anyhow::Error>,
        on_close: mpsc::Sender<()>,
    ) -> Self {
        let server_mode =
            ServerMode { clients: Arc::new(Mutex::new(HashMap::new())), endpoint: config.endpoint, on_message };

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
        let client_id = ClientId::new();

        let client_mode =
            ClientMode { endpoint: config.endpoint.clone(), client_id, on_message, sender: Arc::new(Mutex::new(None)) };

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
        on_message: mpsc::Sender<WsMessage>,
    ) -> Result<(), WsError> {
        // Split the WebSocket stream into sender and receiver parts
        let (mut ws_sender, mut ws_receiver) = ws_stream.split();

        // Create a channel for this client
        let (client_sender, mut client_receiver) = mpsc::channel(32);

        // Register the client in the client registry
        {
            let mut clients = clients.lock().await;
            clients.insert(client_id.clone(), client_sender);
        }

        info!("WebSocket client connected: {}", client_id);

        // Task for forwarding messages from the client channel to the WebSocket
        let client_task = tokio::spawn(async move {
            while let Some(message) = client_receiver.recv().await {
                let json = serde_json::to_string(&message)?;
                if ws_sender.send(Message::Text(json.into())).await.is_err() {
                    // Connection was closed, exit the loop
                    break;
                }
            }
            Ok::<_, WsError>(())
        });

        // Process incoming messages from the WebSocket
        while let Some(result) = ws_receiver.next().await {
            match result {
                Ok(Message::Text(text)) => {
                    debug!("Received WebSocket message: {}", text);
                    match serde_json::from_str::<JsonRpcMessage>(&text) {
                        Ok(message) => {
                            let ws_message = WsMessage { message, client_id: client_id.clone() };
                            if on_message.send(ws_message).await.is_err() {
                                // Channel was closed, exit the loop
                                break;
                            }
                        }
                        Err(err) => {
                            error!("Failed to parse JSON-RPC message: {}", err);
                            // Continue processing other messages despite this error
                        }
                    }
                }
                Ok(Message::Close(_)) => break, // Client initiated close
                _ => {}                         // Ignore other message types
            }
        }

        // Clean up client registration when connection closes
        {
            let mut clients = clients.lock().await;
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
        let clients = clients.lock().await;
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
                            .context(format!("Failed to bind to {}", endpoint_clone))?;

                        info!("WebSocket server listening on {}", endpoint_clone);

                        // Accept and handle incoming connections
                        while let Ok((stream, addr)) = listener.accept().await {
                            let client_id = ClientId::new();
                            debug!("Accepting connection from {} with ID {}", addr, client_id);

                            // Upgrade the TCP stream to a WebSocket stream
                            let ws_stream =
                                accept_async(stream).await.context("Failed to accept WebSocket connection")?;

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
                    let client_id_clone = client.client_id.clone();
                    let on_message_clone = client.on_message.clone();

                    info!("Connecting to WebSocket server at {}", endpoint_clone);

                    // Connect to the server
                    let (ws_stream, _) = connect_async(&endpoint_clone)
                        .await
                        .context(format!("Failed to connect to {}", endpoint_clone))?;

                    // Split the WebSocket stream into sender and receiver parts
                    let (ws_sender, mut ws_receiver) = ws_stream.split();

                    // Store the sender in the client mode for later use
                    {
                        let mut client_sender = client.sender.lock().await;
                        *client_sender = Some(ws_sender);
                    }

                    // Send client ID to the server for identification
                    {
                        let mut sender_guard = client.sender.lock().await;
                        if let Some(ref mut sender) = *sender_guard {
                            let client_id_message = serde_json::json!({
                                "clientId": client_id_clone
                            });

                            sender
                                .send(Message::Text(client_id_message.to_string().into()))
                                .await
                                .context("Failed to send client ID")?;
                        }
                    }

                    // Process incoming messages in a separate task
                    let receive_task = tokio::spawn(async move {
                        while let Some(result) = ws_receiver.next().await {
                            match result {
                                Ok(Message::Text(text)) => {
                                    debug!("Client received [ws]: {}", text);
                                    match serde_json::from_str::<JsonRpcMessage>(&text) {
                                        Ok(message) => {
                                            if on_message_clone.send(message).await.is_err() {
                                                // Channel was closed, exit the loop
                                                break;
                                            }
                                        }
                                        Err(err) => {
                                            error!("Failed to parse JSON-RPC message: {}", err);
                                            // Continue processing other messages despite this error
                                        }
                                    }
                                }
                                Ok(Message::Close(_)) => break, // Server initiated close
                                _ => {}                         // Ignore other message types
                            }
                        }

                        Ok(())
                    });

                    return receive_task.await?;
                }
            }
        });

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
    /// * `metadata` - In server mode, this must contain a client ID. In client mode, this is ignored.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the message was sent successfully, or an error if the message could not be sent.
    ///
    /// # Errors
    ///
    /// Returns an error if the transport is not started, the client is not found, or the message
    /// could not be serialized or sent.
    async fn send(&mut self, message: JsonRpcMessage, metadata: serde_json::Value) -> Result<()> {
        match &*self.mode {
            WsMode::Server(server) => {
                // In server mode, extract the client ID from metadata
                let metadata = serde_json::from_value::<WsMetadata>(metadata)
                    .context("Invalid metadata for WebSocket transport")?;

                Self::send_to_client(&server.clients, &metadata.client_id, message)
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))
            }
            WsMode::Client(client) => {
                // In client mode, send the message to the server
                let mut sender_guard = client.sender.lock().await;
                match &mut *sender_guard {
                    Some(sender) => {
                        let json = serde_json::to_string(&message).context("Failed to serialize JSON-RPC message")?;
                        sender.send(Message::Text(json.into())).await.context("Failed to send WebSocket message")?;
                        Ok(())
                    }
                    None => Err(anyhow::anyhow!("WebSocket connection not established. Call start() first.")),
                }
            }
        }
    }

    /// Closes the WebSocket transport.
    ///
    /// In server mode, this closes all client connections.
    /// In client mode, this closes the connection to the server.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the transport was closed successfully, or an error if the transport
    /// could not be closed.
    async fn close(&mut self) -> Result<()> {
        match &*self.mode {
            WsMode::Server(server) => {
                // In server mode, log closing of client connections
                let clients = server.clients.lock().await;
                for (client_id, _) in clients.iter() {
                    debug!("Closing connection for client {}", client_id);
                    // Note: Actual closing occurs when the server task is aborted
                }
                Ok(())
            }
            WsMode::Client(_) => {
                // In client mode, closing happens when the task is aborted
                Ok(())
            }
        }
    }
}
