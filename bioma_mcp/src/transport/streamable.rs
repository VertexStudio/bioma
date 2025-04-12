use super::{SendMessage, Transport, TransportSender};
use crate::client::StreamableConfig as StreamableClientConfig;
use crate::server::StreamableConfig as StreamableServerConfig;
use crate::transport::Message;
use crate::{ConnectionId, JsonRpcMessage};

use actix_cors::Cors;
use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use anyhow::{Error, Result};
use eventsource_client::{Client, ClientBuilder, SSE};
use futures::{Stream, StreamExt};
use std::collections::HashMap;
use std::net::ToSocketAddrs;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

// Explicitly use the StreamableError for better error handling throughout the code
#[derive(Debug, thiserror::Error)]
enum StreamableError {
    #[error("HTTP error: {0}")]
    HttpError(hyper::StatusCode),

    #[error("Channel error: {0}")]
    ChannelError(String),

    #[error("Connection error: {0}")]
    ConnectionError(String),

    #[error("Client not found: {0}")]
    ClientNotFound(ConnectionId),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("SSE client error: {0}")]
    SseClientError(#[from] eventsource_client::Error),

    #[error("Streamable error: {0}")]
    Other(String),
}

// Implement conversion from anyhow::Error to make integration easier
impl From<anyhow::Error> for StreamableError {
    fn from(err: anyhow::Error) -> Self {
        StreamableError::Other(err.to_string())
    }
}

// Implement conversion from reqwest::Error
impl From<reqwest::Error> for StreamableError {
    fn from(err: reqwest::Error) -> Self {
        if let Some(status) = err.status() {
            StreamableError::HttpError(status)
        } else {
            StreamableError::ConnectionError(err.to_string())
        }
    }
}

enum StreamableMode {
    Server {
        endpoint: String,
        on_message: mpsc::Sender<Message>,
        origins: Vec<String>,
        clients: Arc<Mutex<HashMap<ConnectionId, ClientConnection>>>,
    },

    Client {
        endpoint: String,
        on_message: mpsc::Sender<JsonRpcMessage>,
        sse_stream: bool,
        tx: mpsc::Sender<JsonRpcMessage>,
        rx: Arc<Mutex<mpsc::Receiver<JsonRpcMessage>>>,
    },
}

#[derive(Clone)]
pub struct StreamableTransport {
    mode: Arc<StreamableMode>,
    #[allow(unused)]
    on_error: mpsc::Sender<Error>,
    #[allow(unused)]
    on_close: mpsc::Sender<()>,
}

impl StreamableTransport {
    pub fn new_server(
        config: StreamableServerConfig,
        on_message: mpsc::Sender<Message>,
        on_error: mpsc::Sender<Error>,
        on_close: mpsc::Sender<()>,
    ) -> Self {
        let clients = Arc::new(Mutex::new(HashMap::new()));

        let mode = Arc::new(StreamableMode::Server {
            endpoint: config.endpoint.clone(),
            on_message,
            origins: config.allowed_origins.clone(),
            clients,
        });

        Self { mode, on_error, on_close }
    }

    pub fn new_client(
        config: &StreamableClientConfig,
        on_message: mpsc::Sender<JsonRpcMessage>,
        on_error: mpsc::Sender<Error>,
        on_close: mpsc::Sender<()>,
    ) -> Result<Self> {
        // Create channel for handling responses
        let (tx, rx) = mpsc::channel::<JsonRpcMessage>(32);

        let mode = Arc::new(StreamableMode::Client {
            endpoint: config.endpoint.clone(),
            on_message,
            sse_stream: config.sse_stream,
            tx,
            rx: Arc::new(Mutex::new(rx)),
        });

        Ok(Self { mode, on_error, on_close })
    }
}

impl Transport for StreamableTransport {
    async fn start(&mut self) -> Result<JoinHandle<Result<()>>> {
        match &*self.mode {
            StreamableMode::Server { endpoint, on_message, origins, clients } => {
                let on_message = on_message.clone();
                let shared_clients = clients.clone();

                let app_data = web::Data::new(AppState { on_message, clients: shared_clients });
                let origins_list = origins.clone();

                let socket_addr = endpoint
                    .replace("http://", "")
                    .replace("https://", "")
                    .to_socket_addrs()?
                    .next()
                    .ok_or_else(|| StreamableError::ConnectionError("Failed to parse endpoint URL".to_string()))?;

                let server = HttpServer::new(move || {
                    let cors = Cors::default()
                        .allowed_methods(vec!["GET", "POST"])
                        .allowed_headers(vec!["content-type", "accept"])
                        .max_age(3600);

                    let mut cors_builder = cors;
                    for origin in &origins_list {
                        cors_builder = cors_builder.allowed_origin(origin);
                    }

                    App::new()
                        .wrap(cors_builder)
                        .app_data(app_data.clone())
                        .route("/", web::get().to(get_handler))
                        .route("/", web::post().to(post_handler))
                })
                .bind(socket_addr)?
                .run();

                let handle = tokio::spawn(async move {
                    info!("Starting streamable server on {}", socket_addr);
                    server.await.map_err(|e| anyhow::anyhow!("Server error: {}", e))
                });

                Ok(handle)
            }
            StreamableMode::Client { endpoint, on_message, sse_stream, rx, .. } => {
                let endpoint_clone = endpoint.clone();
                let on_message_clone = on_message.clone();
                let sse_stream_flag = *sse_stream;

                let rx_arc = rx.clone();

                let handle = tokio::spawn(async move {
                    if sse_stream_flag {
                        let sse_endpoint = endpoint_clone.clone();
                        let sse_on_message = on_message_clone.clone();

                        // Use a dedicated SSE client library instead of manual parsing
                        tokio::spawn(async move {
                            debug!("Initiating SSE connection to {}", sse_endpoint);

                            // Using eventsource-client for SSE handling
                            match setup_sse_client(&sse_endpoint, sse_on_message).await {
                                Ok(_) => debug!("SSE client completed successfully"),
                                Err(e) => error!("SSE client error: {}", e),
                            }
                        });
                    }

                    let mut rx_guard = rx_arc.lock().await;
                    while let Some(message) = rx_guard.recv().await {
                        if let Err(e) = on_message_clone.send(message).await {
                            error!("Failed to forward message: {}", e);
                        }
                    }

                    Ok(())
                });

                Ok(handle)
            }
        }
    }

    async fn send(&mut self, message: JsonRpcMessage, conn_id: ConnectionId) -> Result<()> {
        match &*self.mode {
            StreamableMode::Server { clients, .. } => {
                let clients_guard = clients.lock().await;
                if let Some(client) = clients_guard.get(&conn_id) {
                    client.sender.send(message).await.map_err(|e| {
                        StreamableError::ChannelError(format!("Failed to send message to client {}: {}", conn_id, e))
                    })?;
                    Ok(())
                } else {
                    Err(StreamableError::ClientNotFound(conn_id).into())
                }
            }
            StreamableMode::Client { endpoint, tx, .. } => {
                let json = serde_json::to_string(&message).map_err(StreamableError::SerializationError)?;

                let endpoint = endpoint.clone();
                let tx = tx.clone();
                let client = reqwest::Client::new();

                tokio::spawn(async move {
                    match client
                        .post(endpoint)
                        .header("Accept", "application/json, text/event-stream")
                        .body(json)
                        .send()
                        .await
                    {
                        Ok(response) => {
                            if response.status().is_success() {
                                match response.text().await {
                                    Ok(text) => {
                                        if !text.is_empty() {
                                            match serde_json::from_str::<JsonRpcMessage>(&text) {
                                                Ok(message) => {
                                                    if let Err(e) = tx.send(message).await {
                                                        error!("Failed to forward message: {}", e);
                                                    }
                                                }
                                                Err(e) => {
                                                    error!("Failed to parse JSON-RPC message: {}", e);
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        error!("Failed to get response text: {}", e);
                                    }
                                }
                            } else {
                                error!("Request failed with status: {}", response.status());
                            }
                        }
                        Err(e) => {
                            error!("Request error: {}", e);
                        }
                    }
                });

                Ok(())
            }
        }
    }

    fn close(&mut self) -> impl std::future::Future<Output = Result<()>> {
        async move { todo!() }
    }

    fn sender(&self) -> TransportSender {
        TransportSender::new_streamable(StreamableTransportSender { mode: self.mode.clone() })
    }
}

// New function to setup SSE client using eventsource-client crate
async fn setup_sse_client(endpoint: &str, on_message: mpsc::Sender<JsonRpcMessage>) -> Result<(), StreamableError> {
    let client = ClientBuilder::for_url(endpoint)?.header("Accept", "text/event-stream")?.build();

    let mut event_stream = client.stream();

    tokio::spawn(async move {
        while let Some(event_result) = event_stream.next().await {
            match event_result {
                Ok(SSE::Event(event)) => {
                    if let Ok(json_message) = serde_json::from_str::<JsonRpcMessage>(&event.data) {
                        if let Err(e) = on_message.send(json_message).await {
                            error!("Failed to forward SSE message: {}", e);
                            break;
                        }
                    } else {
                        error!("Failed to parse SSE message data as JsonRpcMessage");
                    }
                }
                Ok(SSE::Connected(_)) => {
                    debug!("SSE connection opened");
                }
                Err(e) => {
                    error!("SSE stream error: {}", e);
                    break;
                }
                _ => {}
            }
        }
        debug!("SSE stream ended");
    });

    Ok(())
}

struct ClientConnection {
    sender: mpsc::Sender<JsonRpcMessage>,
}

struct AppState {
    on_message: mpsc::Sender<Message>,
    clients: Arc<Mutex<HashMap<ConnectionId, ClientConnection>>>,
}

struct SseStream {
    rx: mpsc::Receiver<JsonRpcMessage>,
}

impl Stream for SseStream {
    type Item = Result<web::Bytes, actix_web::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        match self.rx.poll_recv(cx) {
            Poll::Ready(Some(msg)) => match serde_json::to_string(&msg) {
                Ok(json) => {
                    let event = format!("data: {}\n\n", json);
                    Poll::Ready(Some(Ok(web::Bytes::from(event))))
                }
                Err(e) => {
                    error!("Failed to serialize message: {}", e);
                    Poll::Ready(Some(Err(actix_web::error::ErrorInternalServerError("Serialization error"))))
                }
            },
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

async fn get_handler(req: HttpRequest, app_state: web::Data<AppState>) -> impl Responder {
    if req.headers().get("Accept").map_or(false, |h| h.to_str().unwrap_or("").contains("text/event-stream")) {
        let (tx, rx) = mpsc::channel::<JsonRpcMessage>(32);

        let conn_id = ConnectionId::new(Some(uuid::Uuid::new_v4().to_string()));

        let client = ClientConnection { sender: tx.clone() };

        app_state.clients.lock().await.insert(conn_id.clone(), client);

        let sse_stream = SseStream { rx };

        HttpResponse::Ok()
            .content_type("text/event-stream")
            .append_header(("Cache-Control", "no-cache"))
            .append_header(("Connection", "keep-alive"))
            .streaming(sse_stream)
    } else {
        HttpResponse::BadRequest().body("Invalid request")
    }
}

async fn post_handler(payload: web::Json<JsonRpcMessage>, app_state: web::Data<AppState>) -> impl Responder {
    let message = Message { conn_id: ConnectionId::new(None), message: payload.into_inner() };

    match app_state.on_message.send(message).await {
        Ok(_) => HttpResponse::Accepted().finish(),
        Err(e) => {
            error!("Failed to forward message: {}", e);
            HttpResponse::InternalServerError().body("Failed to process message")
        }
    }
}

#[derive(Clone)]
pub struct StreamableTransportSender {
    mode: Arc<StreamableMode>,
}

impl SendMessage for StreamableTransportSender {
    async fn send(&self, message: JsonRpcMessage, conn_id: ConnectionId) -> Result<()> {
        match &*self.mode {
            StreamableMode::Server { clients, .. } => {
                let clients_guard = clients.lock().await;
                if let Some(client) = clients_guard.get(&conn_id) {
                    client.sender.send(message).await.map_err(|e| {
                        StreamableError::ChannelError(format!("Failed to send message to client {}: {}", conn_id, e))
                    })?;
                    Ok(())
                } else {
                    Err(StreamableError::ClientNotFound(conn_id).into())
                }
            }
            StreamableMode::Client { endpoint, tx, .. } => {
                let json = serde_json::to_string(&message).map_err(StreamableError::SerializationError)?;

                let endpoint = endpoint.clone();
                let tx = tx.clone();

                let res = reqwest::Client::new()
                    .post(endpoint)
                    .header("Accept", "application/json, text/event-stream")
                    .body(json)
                    .send()
                    .await
                    .map_err(StreamableError::from)?;

                if res.status().is_success() {
                    if let Ok(response_body) = res.text().await {
                        if !response_body.is_empty() {
                            if let Ok(response_message) = serde_json::from_str::<JsonRpcMessage>(&response_body) {
                                if let Err(e) = tx.send(response_message).await {
                                    return Err(StreamableError::ChannelError(format!(
                                        "Failed to send response through channel: {}",
                                        e
                                    ))
                                    .into());
                                }
                            }
                        }
                    }
                    Ok(())
                } else {
                    Err(StreamableError::HttpError(res.status()).into())
                }
            }
        }
    }
}
