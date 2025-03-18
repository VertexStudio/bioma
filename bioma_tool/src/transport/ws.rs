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
use tokio_tungstenite::{accept_async, connect_async, tungstenite::protocol::Message, WebSocketStream};
use tracing::{debug, error, info};

#[derive(Debug, Error)]
enum WsError {
    #[error("WebSocket connection error: {0}")]
    ConnectionError(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("JSON serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Client not found: {0}")]
    ClientNotFound(ClientId),

    #[error("Message sending failed: {0}")]
    SendError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Metadata for WebSocket transport
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsMetadata {
    pub client_id: ClientId,
}

/// WebSocket message with client identification
#[derive(Debug, Clone)]
pub struct WsMessage {
    pub message: JsonRpcMessage,
    pub client_id: ClientId,
}

type WsStream = WebSocketStream<TcpStream>;
type ClientRegistry = Arc<Mutex<HashMap<ClientId, mpsc::Sender<JsonRpcMessage>>>>;

/// WebSocket transport mode enumeration
enum WsMode {
    /// Server mode with connected clients registry
    Server { clients: ClientRegistry, endpoint: String, on_message: mpsc::Sender<WsMessage> },

    /// Client mode connecting to a server
    Client { endpoint: String, client_id: ClientId, on_message: mpsc::Sender<JsonRpcMessage> },
}

/// WebSocket transport implementation
#[derive(Clone)]
pub struct WsTransport {
    mode: Arc<WsMode>,
    #[allow(unused)]
    on_error: mpsc::Sender<anyhow::Error>,
    #[allow(unused)]
    on_close: mpsc::Sender<()>,
}

impl WsTransport {
    /// Creates a new WebSocket transport in server mode
    ///
    /// # Arguments
    ///
    /// * `config` - Server configuration parameters
    /// * `on_message` - Channel to forward received messages
    /// * `on_error` - Channel to report errors
    /// * `on_close` - Channel to signal connection closure
    ///
    /// # Returns
    ///
    /// A new WsTransport instance configured in server mode
    pub fn new_server(
        config: WsServerConfig,
        on_message: mpsc::Sender<WsMessage>,
        on_error: mpsc::Sender<anyhow::Error>,
        on_close: mpsc::Sender<()>,
    ) -> Self {
        Self {
            mode: Arc::new(WsMode::Server {
                clients: Arc::new(Mutex::new(HashMap::new())),
                endpoint: config.endpoint,
                on_message,
            }),
            on_error,
            on_close,
        }
    }

    /// Creates a new WebSocket transport in client mode
    ///
    /// # Arguments
    ///
    /// * `config` - Client configuration parameters
    /// * `on_message` - Channel to forward received messages
    /// * `on_error` - Channel to report errors
    /// * `on_close` - Channel to signal connection closure
    ///
    /// # Returns
    ///
    /// A Result containing either a new WsTransport instance configured in client mode
    /// or an error if the configuration is invalid
    pub fn new_client(
        config: &WsClientConfig,
        on_message: mpsc::Sender<JsonRpcMessage>,
        on_error: mpsc::Sender<anyhow::Error>,
        on_close: mpsc::Sender<()>,
    ) -> Result<Self> {
        let client_id = ClientId::new();

        Ok(Self {
            mode: Arc::new(WsMode::Client { endpoint: config.endpoint.clone(), client_id, on_message }),
            on_error,
            on_close,
        })
    }

    /// Handles a client WebSocket connection
    ///
    /// # Arguments
    ///
    /// * `ws_stream` - The WebSocket stream for the client
    /// * `client_id` - The client's unique identifier
    /// * `clients` - Registry of connected clients
    /// * `on_message` - Channel to forward received messages
    ///
    /// # Returns
    ///
    /// A result indicating success or failure
    async fn handle_client_connection(
        ws_stream: WsStream,
        client_id: ClientId,
        clients: ClientRegistry,
        on_message: mpsc::Sender<WsMessage>,
    ) -> Result<(), WsError> {
        let (mut ws_sender, mut ws_receiver) = ws_stream.split();

        // Create a channel for this client
        let (client_sender, mut client_receiver) = mpsc::channel(32);

        // Register the client
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
                                break;
                            }
                        }
                        Err(err) => {
                            error!("Failed to parse JSON-RPC message: {}", err);
                        }
                    }
                }
                Ok(Message::Close(_)) => break,
                _ => {}
            }
        }

        // Clean up client registration
        {
            let mut clients = clients.lock().await;
            clients.remove(&client_id);
        }

        info!("WebSocket client disconnected: {}", client_id);
        client_task.abort();
        Ok(())
    }

    /// Sends a message to a specific client
    ///
    /// # Arguments
    ///
    /// * `clients` - Registry of connected clients
    /// * `client_id` - The target client's unique identifier
    /// * `message` - The message to send
    ///
    /// # Returns
    ///
    /// A result indicating success or failure
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
    async fn start(&mut self) -> Result<JoinHandle<Result<()>>> {
        let mode = self.mode.clone();

        let handle = tokio::spawn(async move {
            match &*mode {
                WsMode::Server { clients, endpoint, on_message } => {
                    let listener =
                        TcpListener::bind(endpoint).await.context(format!("Failed to bind to {}", endpoint))?;

                    info!("WebSocket server listening on {}", endpoint);

                    while let Ok((stream, addr)) = listener.accept().await {
                        let client_id = ClientId::new();
                        debug!("Accepting connection from {} with ID {}", addr, client_id);

                        let ws_stream = accept_async(stream).await.context("Failed to accept WebSocket connection")?;

                        let clients = clients.clone();
                        let on_message = on_message.clone();
                        let client_id_clone = client_id.clone();

                        tokio::spawn(async move {
                            if let Err(e) =
                                Self::handle_client_connection(ws_stream, client_id_clone, clients, on_message).await
                            {
                                error!("Error handling WebSocket connection: {}", e);
                            }
                        });
                    }

                    Ok(())
                }
                WsMode::Client { endpoint, client_id, on_message } => {
                    info!("Connecting to WebSocket server at {}", endpoint);

                    let (ws_stream, _) =
                        connect_async(endpoint).await.context(format!("Failed to connect to {}", endpoint))?;

                    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

                    // Send client ID to the server
                    let client_id_message = serde_json::json!({
                        "clientId": client_id
                    });
                    ws_sender
                        .send(Message::Text(client_id_message.to_string().into()))
                        .await
                        .context("Failed to send client ID")?;

                    // Process incoming messages
                    while let Some(result) = ws_receiver.next().await {
                        match result {
                            Ok(Message::Text(text)) => {
                                debug!("Client received [ws]: {}", text);
                                match serde_json::from_str::<JsonRpcMessage>(&text) {
                                    Ok(message) => {
                                        if on_message.send(message).await.is_err() {
                                            break;
                                        }
                                    }
                                    Err(err) => {
                                        error!("Failed to parse JSON-RPC message: {}", err);
                                    }
                                }
                            }
                            Ok(Message::Close(_)) => break,
                            _ => {}
                        }
                    }

                    Ok(())
                }
            }
        });

        Ok(handle)
    }

    async fn send(&mut self, message: JsonRpcMessage, metadata: serde_json::Value) -> Result<()> {
        match &*self.mode {
            WsMode::Server { clients, .. } => {
                let metadata = serde_json::from_value::<WsMetadata>(metadata)
                    .context("Invalid metadata for WebSocket transport")?;

                Self::send_to_client(clients, &metadata.client_id, message).await.map_err(|e| anyhow::anyhow!("{}", e))
            }
            WsMode::Client { .. } => {
                let json = serde_json::to_string(&message).context("Failed to serialize JSON-RPC message")?;

                match &*self.mode {
                    WsMode::Client { endpoint, .. } => {
                        let (mut ws_stream, _) =
                            connect_async(endpoint).await.context(format!("Failed to connect to {}", endpoint))?;

                        ws_stream.send(Message::Text(json.into())).await.context("Failed to send WebSocket message")?;

                        Ok(())
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    async fn close(&mut self) -> Result<()> {
        match &*self.mode {
            WsMode::Server { clients, .. } => {
                let clients = clients.lock().await;
                for (client_id, _) in clients.iter() {
                    debug!("Closing connection for client {}", client_id);
                }
                Ok(())
            }
            WsMode::Client { .. } => Ok(()),
        }
    }
}
