use super::{SendMessage, Transport, TransportSender};

use crate::client::WsConfig as WsClientConfig;
use crate::server::WsConfig as WsServerConfig;
use crate::transport::Message;
use crate::{ConnectionId, JsonRpcMessage};

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::{
    accept_async, connect_async,
    tungstenite::protocol::{frame::coding::CloseCode, CloseFrame, Message as WsMessage},
    WebSocketStream,
};
use tracing::{debug, error, info, warn};

#[derive(Debug, Error)]
pub enum WsError {
    #[error("JSON serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Client not found: {0}")]
    ClientNotFound(ConnectionId),

    #[error("Message sending failed: {0}")]
    SendError(String),

    #[error("WebSocket connection error: {0}")]
    ConnectionError(String),

    #[error("Network binding error: {0}")]
    BindError(String),

    #[error("WebSocket protocol error: {0}")]
    ProtocolError(String),

    #[error("Transport state error: {0}")]
    StateError(String),

    #[error("Message parsing error: {0}")]
    MessageError(String),

    #[error("Timeout error: {0}")]
    TimeoutError(String),

    #[error("WebSocket handshake failed: {0}")]
    HandshakeError(String),

    #[error("Invalid metadata: {0}")]
    MetadataError(String),
}

type WsStreamServer = WebSocketStream<TcpStream>;

type WsStreamClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

type ClientRegistry = Arc<RwLock<HashMap<ConnectionId, mpsc::Sender<JsonRpcMessage>>>>;

type WsSenderClient = Arc<Mutex<Option<futures_util::stream::SplitSink<WsStreamClient, WsMessage>>>>;

struct ServerMode {
    clients: ClientRegistry,

    endpoint: String,

    on_message: mpsc::Sender<Message>,
}

struct ClientMode {
    endpoint: String,

    on_message: mpsc::Sender<JsonRpcMessage>,

    sender: WsSenderClient,

    close_handshake: Arc<Mutex<Option<oneshot::Receiver<()>>>>,
}

enum WsMode {
    Server(ServerMode),

    Client(ClientMode),
}

#[derive(Clone)]
pub struct WsTransport {
    mode: Arc<WsMode>,

    #[allow(unused)]
    on_error: mpsc::Sender<anyhow::Error>,

    #[allow(unused)]
    on_close: mpsc::Sender<()>,
}

#[derive(Clone)]
pub struct WsTransportSender {
    mode: Arc<WsMode>,
}

impl SendMessage for WsTransportSender {
    async fn send(&self, message: JsonRpcMessage, conn_id: ConnectionId) -> Result<()> {
        match &*self.mode {
            WsMode::Server(server) => WsTransport::send_to_client(&server.clients, &conn_id, message)
                .await
                .map_err(|e| anyhow::Error::msg(format!("Failed to send to client: {}", e))),
            WsMode::Client(client) => {
                let json = serde_json::to_string(&message)
                    .map_err(|e| anyhow::Error::msg(format!("Serialization error: {}", e)))?;

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

    pub fn new_client(
        config: &WsClientConfig,
        on_message: mpsc::Sender<JsonRpcMessage>,
        on_error: mpsc::Sender<anyhow::Error>,
        on_close: mpsc::Sender<()>,
    ) -> Result<Self> {
        let client_mode = ClientMode {
            endpoint: config.endpoint.clone(),
            on_message,
            sender: Arc::new(Mutex::new(None)),
            close_handshake: Arc::new(Mutex::new(None)),
        };

        Ok(Self { mode: Arc::new(WsMode::Client(client_mode)), on_error, on_close })
    }

    async fn handle_client_connection(
        ws_stream: WsStreamServer,
        conn_id: ConnectionId,
        clients: ClientRegistry,
        on_message: mpsc::Sender<Message>,
    ) -> Result<(), WsError> {
        let (mut ws_sender, mut ws_receiver) = ws_stream.split();

        let (client_sender, mut client_receiver) = mpsc::channel(32);

        {
            let mut clients = clients.write().await;
            clients.insert(conn_id.clone(), client_sender);
        }

        info!("WebSocket client connected: {}", conn_id);

        let client_task = tokio::spawn(async move {
            while let Some(message) = client_receiver.recv().await {
                let json = serde_json::to_string(&message)?;
                if ws_sender.send(WsMessage::Text(json.into())).await.is_err() {
                    break;
                }
            }
            Ok::<_, WsError>(())
        });

        while let Some(result) = ws_receiver.next().await {
            match result {
                Ok(WsMessage::Text(text)) => {
                    debug!("Received WebSocket message: {}", text);
                    match serde_json::from_str::<JsonRpcMessage>(&text) {
                        Ok(message) => {
                            let ws_message = Message { conn_id: conn_id.clone(), message };
                            if on_message.send(ws_message).await.is_err() {
                                break;
                            }
                        }
                        Err(err) => {
                            let error = WsError::MessageError(format!("Failed to parse JSON-RPC message: {}", err));
                            error!("{}", error);
                        }
                    }
                }
                Ok(WsMessage::Close(_)) => break,
                Err(err) => {
                    error!("WebSocket connection error: {}", err);
                    break;
                }
                _ => {}
            }
        }

        {
            let mut clients = clients.write().await;
            clients.remove(&conn_id);
        }

        info!("WebSocket client disconnected: {}", conn_id);
        client_task.abort();
        Ok(())
    }

    async fn send_to_client(
        clients: &ClientRegistry,
        conn_id: &ConnectionId,
        message: JsonRpcMessage,
    ) -> Result<(), WsError> {
        let clients = clients.read().await;
        if let Some(sender) = clients.get(conn_id) {
            sender
                .send(message)
                .await
                .map_err(|_| WsError::SendError(format!("Failed to send message to client {}", conn_id)))?;
            Ok(())
        } else {
            Err(WsError::ClientNotFound(conn_id.clone()))
        }
    }
}

impl Transport for WsTransport {
    async fn start(&mut self) -> Result<JoinHandle<Result<()>>> {
        let mode = self.mode.clone();

        let (sender_ready_tx, sender_ready_rx) = tokio::sync::oneshot::channel();
        let sender_ready_tx = Arc::new(Mutex::new(Some(sender_ready_tx)));

        let handle = tokio::spawn(async move {
            match &*mode {
                WsMode::Server(server) => {
                    let clients_clone = server.clients.clone();
                    let on_message_clone = server.on_message.clone();
                    let endpoint_clone = server.endpoint.clone();

                    let server_handle = tokio::spawn(async move {
                        let listener = TcpListener::bind(&endpoint_clone)
                            .await
                            .map_err(|e| WsError::BindError(format!("Failed to bind to {}: {}", endpoint_clone, e)))?;

                        info!("WebSocket server listening on {}", endpoint_clone);

                        while let Ok((stream, addr)) = listener.accept().await {
                            let conn_id = ConnectionId::new(None);
                            debug!("Accepting connection from {} with ID {}", addr, conn_id);

                            let ws_stream = accept_async(stream).await.map_err(|e| {
                                WsError::HandshakeError(format!("Failed to accept WebSocket connection: {}", e))
                            })?;

                            let clients = clients_clone.clone();
                            let on_message = on_message_clone.clone();
                            let client_id_clone = conn_id.clone();

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
                    let close_handshake_clone = client.close_handshake.clone();

                    info!("Connecting to WebSocket server at {}", endpoint_clone);

                    let (ws_stream, _) = connect_async(&endpoint_clone).await.map_err(|e| {
                        WsError::ConnectionError(format!("Failed to connect to {}: {}", endpoint_clone, e))
                    })?;

                    let (ws_sender, mut ws_receiver) = ws_stream.split();

                    {
                        let mut client_sender = sender_clone.lock().await;
                        *client_sender = Some(ws_sender);

                        if let Some(tx) = sender_ready.lock().await.take() {
                            let _ = tx.send(());
                        }
                    }

                    let (close_tx, close_rx) = oneshot::channel();

                    {
                        let mut handshake = close_handshake_clone.lock().await;
                        *handshake = Some(close_rx);
                    }

                    let receive_task = tokio::spawn(async move {
                        while let Some(result) = ws_receiver.next().await {
                            match result {
                                Ok(WsMessage::Text(text)) => {
                                    debug!("Client received [ws]: {}", text);
                                    match serde_json::from_str::<JsonRpcMessage>(&text) {
                                        Ok(message) => {
                                            if on_message_clone.send(message).await.is_err() {
                                                break;
                                            }
                                        }
                                        Err(err) => {
                                            let error = WsError::MessageError(format!(
                                                "Failed to parse JSON-RPC message: {}",
                                                err
                                            ));
                                            error!("{}", error);
                                        }
                                    }
                                }
                                Ok(WsMessage::Close(_)) => {
                                    debug!("Received close frame from server, close handshake complete");

                                    let _ = close_tx.send(());
                                    break;
                                }
                                Err(err) => {
                                    let err_str = err.to_string();
                                    if err_str.contains("Connection reset") {
                                        debug!("Connection reset during close handshake (expected)");

                                        let _ = close_tx.send(());
                                    } else {
                                        error!("WebSocket connection error: {}", err);
                                    }
                                    break;
                                }
                                _ => {}
                            }
                        }

                        Ok(())
                    });

                    return receive_task.await?;
                }
            }
        });

        if let WsMode::Client(_) = &*self.mode {
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

    async fn send(&mut self, message: JsonRpcMessage, conn_id: ConnectionId) -> Result<()> {
        match &*self.mode {
            WsMode::Server(server) => {
                debug!("Server sending WebSocket message");
                Self::send_to_client(&server.clients, &conn_id, message).await.map_err(|e| anyhow::anyhow!("{}", e))
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

    async fn close(&mut self) -> Result<()> {
        match &*self.mode {
            WsMode::Server(server) => {
                let mut clients = server.clients.write().await;
                let client_count = clients.len();

                if client_count > 0 {
                    info!("Initiating shutdown for {} client connections", client_count);

                    clients.clear();
                    debug!("Cleared client registry, connection handlers will clean up");
                } else {
                    debug!("No clients to close");
                }

                info!("All client connections marked for closure");

                Ok(())
            }
            WsMode::Client(client) => {
                let mut sender_guard = client.sender.lock().await;

                if let Some(ref mut sender) = *sender_guard {
                    info!("Initiating graceful shutdown of WebSocket connection");

                    let close_handshake_rx = {
                        let mut handshake = client.close_handshake.lock().await;
                        handshake.take()
                    };

                    let close_frame = WsMessage::Close(Some(CloseFrame {
                        code: CloseCode::Normal,
                        reason: "Client initiated shutdown".into(),
                    }));

                    match sender.send(close_frame).await {
                        Ok(_) => {
                            debug!("WebSocket close frame sent successfully");

                            drop(sender_guard);

                            if let Some(rx) = close_handshake_rx {
                                match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
                                    Ok(Ok(())) => debug!("WebSocket close handshake completed successfully"),
                                    Ok(Err(_)) => warn!("Close handshake receiver dropped without sending"),
                                    Err(_) => warn!("Timeout waiting for WebSocket close handshake"),
                                }
                            }

                            if let Ok(mut sender_guard) = client.sender.try_lock() {
                                *sender_guard = None;
                            }
                        }
                        Err(e) => {
                            error!("Error sending WebSocket close frame: {}", e);
                            *sender_guard = None;
                        }
                    }
                } else {
                    debug!("WebSocket connection already closed");
                }

                Ok(())
            }
        }
    }

    fn sender(&self) -> TransportSender {
        TransportSender::new_ws(WsTransportSender { mode: self.mode.clone() })
    }
}
