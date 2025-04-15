use super::{SendMessage, Transport, TransportSender};
use crate::client::StreamableConfig as StreamableClientConfig;
use crate::server::StreamableConfig as StreamableServerConfig;
use crate::transport::Message;
use crate::{ConnectionId, JsonRpcMessage};

use actix_cors::Cors;
use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use anyhow::{Error, Result};
use futures::Stream;
use futures::StreamExt;
use std::collections::HashMap;
use std::net::ToSocketAddrs;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

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

impl From<anyhow::Error> for StreamableError {
    fn from(err: anyhow::Error) -> Self {
        StreamableError::Other(err.to_string())
    }
}

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
        sse_streams: Arc<Mutex<HashMap<ConnectionId, mpsc::Sender<JsonRpcMessage>>>>,
        pending_requests: Arc<Mutex<HashMap<ConnectionId, oneshot::Sender<JsonRpcMessage>>>>,
    },

    Client {
        endpoint: String,
        on_message: mpsc::Sender<JsonRpcMessage>,
        tx: mpsc::Sender<JsonRpcMessage>,
        rx: Arc<Mutex<mpsc::Receiver<JsonRpcMessage>>>,
        session_id: Arc<RwLock<Option<String>>>,
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
        let sse_streams = Arc::new(Mutex::new(HashMap::new()));
        let pending_requests = Arc::new(Mutex::new(HashMap::new()));
        let mode = Arc::new(StreamableMode::Server {
            endpoint: config.endpoint.clone(),
            on_message,
            origins: config.allowed_origins.clone(),
            sse_streams,
            pending_requests,
        });

        Self { mode, on_error, on_close }
    }

    pub fn new_client(
        config: &StreamableClientConfig,
        on_message: mpsc::Sender<JsonRpcMessage>,
        on_error: mpsc::Sender<Error>,
        on_close: mpsc::Sender<()>,
    ) -> Result<Self> {
        let (tx, rx) = mpsc::channel::<JsonRpcMessage>(32);

        let mode = Arc::new(StreamableMode::Client {
            endpoint: config.endpoint.clone(),
            on_message,
            tx,
            rx: Arc::new(Mutex::new(rx)),
            session_id: Arc::new(RwLock::new(None)),
        });

        Ok(Self { mode, on_error, on_close })
    }
}

impl Transport for StreamableTransport {
    async fn start(&mut self) -> Result<JoinHandle<Result<()>>> {
        match &*self.mode {
            StreamableMode::Server { endpoint, on_message, origins, sse_streams, pending_requests } => {
                let on_message = on_message.clone();
                let shared_sse_streams = sse_streams.clone();
                let shared_pending_requests = pending_requests.clone();

                let app_data = web::Data::new(AppState {
                    on_message,
                    sse_streams: shared_sse_streams,
                    pending_requests: shared_pending_requests,
                });
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
            StreamableMode::Client { on_message, rx, .. } => {
                let on_message_clone = on_message.clone();

                let rx_arc = rx.clone();

                let handle = tokio::spawn(async move {
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
            StreamableMode::Server { sse_streams, .. } => {
                let sse_streams_guard = sse_streams.lock().await;
                if let Some(client) = sse_streams_guard.get(&conn_id) {
                    client.send(message).await.map_err(|e| {
                        StreamableError::ChannelError(format!("Failed to send message to client {}: {}", conn_id, e))
                    })?;
                    Ok(())
                } else {
                    Err(StreamableError::ClientNotFound(conn_id).into())
                }
            }
            StreamableMode::Client { endpoint, tx, session_id, .. } => {
                let json = serde_json::to_string(&message)?;
                let endpoint = endpoint.clone();
                let tx = tx.clone();
                let client = reqwest::Client::new();

                let is_initialize_request = match &message {
                    JsonRpcMessage::Request(request) => match request {
                        jsonrpc_core::Request::Single(jsonrpc_core::Call::MethodCall(method_call)) => {
                            method_call.method == "initialize"
                        }
                        _ => false,
                    },
                    _ => false,
                };

                let mut request =
                    client.post(endpoint.clone()).header("Accept", "application/json, text/event-stream").body(json);

                if let Some(id) = session_id.read().await.as_ref() {
                    request = request.header("Mcp-Session-Id", id);
                }

                let response = request.send().await.map_err(StreamableError::from)?;

                if response.status().is_success() {
                    if is_initialize_request {
                        if let Some(header_value) = response.headers().get("Mcp-Session-Id") {
                            if let Ok(id) = header_value.to_str() {
                                *session_id.write().await = Some(id.to_string());
                                debug!("Saved MCP session ID: {}", id);

                                let sse_endpoint = endpoint.clone();
                                let tx_clone = tx.clone();
                                let session_id_value = id.to_string();
                                let client = reqwest::Client::new();

                                tokio::spawn(async move {
                                    let sse_request = client
                                        .get(sse_endpoint)
                                        .header("Accept", "text/event-stream")
                                        .header("Mcp-Session-Id", session_id_value);

                                    match sse_request.send().await {
                                        Ok(sse_response) => {
                                            if sse_response.status().is_success() {
                                                debug!("SSE stream established successfully");

                                                if let Err(e) = setup_sse_client(sse_response, tx_clone).await {
                                                    error!("Error setting up SSE client: {}", e);
                                                }
                                            } else if sse_response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
                                                debug!("Server does not support SSE (405 Method Not Allowed), continuing normally");
                                            } else {
                                                error!("Failed to establish SSE stream: {}", sse_response.status());
                                            }
                                        }
                                        Err(e) => {
                                            error!("Error sending SSE request: {}", e);
                                        }
                                    }
                                });
                            }
                        }
                    }

                    if let Ok(response_body) = response.text().await {
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
                    Err(StreamableError::HttpError(response.status()).into())
                }
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

struct AppState {
    on_message: mpsc::Sender<Message>,
    sse_streams: Arc<Mutex<HashMap<ConnectionId, mpsc::Sender<JsonRpcMessage>>>>,
    pending_requests: Arc<Mutex<HashMap<ConnectionId, oneshot::Sender<JsonRpcMessage>>>>,
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

        app_state.sse_streams.lock().await.insert(conn_id.clone(), tx);

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
    let message_content = payload.into_inner();
    let conn_id = ConnectionId::new(None);

    match &message_content {
        JsonRpcMessage::Response(_) => {
            match app_state.on_message.send(Message { conn_id: conn_id.clone(), message: message_content }).await {
                Ok(_) => HttpResponse::Accepted().finish(),
                Err(e) => {
                    error!("Failed to forward response message: {}", e);
                    HttpResponse::InternalServerError().body("Failed to process message")
                }
            }
        }
        JsonRpcMessage::Request(request) => {
            let is_notification = match request {
                jsonrpc_core::Request::Single(jsonrpc_core::Call::Notification(_)) => true,
                jsonrpc_core::Request::Batch(calls)
                    if calls.iter().all(|c| matches!(c, jsonrpc_core::Call::Notification(_))) =>
                {
                    true
                }
                _ => false,
            };

            if is_notification {
                match app_state.on_message.send(Message { conn_id: conn_id.clone(), message: message_content }).await {
                    Ok(_) => {
                        return HttpResponse::Accepted().finish();
                    }
                    Err(e) => {
                        error!("Failed to forward notification message: {}", e);
                        return HttpResponse::InternalServerError().body("Failed to process message");
                    }
                }
            }

            let (tx, rx) = oneshot::channel();

            app_state.pending_requests.lock().await.insert(conn_id.clone(), tx);

            match app_state.on_message.send(Message { conn_id: conn_id.clone(), message: message_content }).await {
                Ok(_) => match rx.await {
                    Ok(response) => HttpResponse::Ok().json(response),
                    Err(e) => {
                        error!("Failed to receive response: {}", e);
                        HttpResponse::InternalServerError().body("Failed to receive response")
                    }
                },
                Err(e) => {
                    error!("Failed to forward message: {}", e);
                    HttpResponse::InternalServerError().body("Failed to process message")
                }
            }
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
            StreamableMode::Server { sse_streams, pending_requests, .. } => {
                let sse_streams_guard = sse_streams.lock().await;
                if let Some(client) = sse_streams_guard.get(&conn_id) {
                    client.send(message).await.map_err(|e| {
                        StreamableError::ChannelError(format!("Failed to send message to client {}: {}", conn_id, e))
                    })?;
                    Ok(())
                } else {
                    Err(StreamableError::ClientNotFound(conn_id).into())
                }
            }
            StreamableMode::Client { endpoint, tx, session_id, .. } => {
                let json = serde_json::to_string(&message)?;
                let endpoint = endpoint.clone();
                let tx = tx.clone();
                let client = reqwest::Client::new();

                let is_initialize_request = match &message {
                    JsonRpcMessage::Request(request) => match request {
                        jsonrpc_core::Request::Single(jsonrpc_core::Call::MethodCall(method_call)) => {
                            method_call.method == "initialize"
                        }
                        _ => false,
                    },
                    _ => false,
                };

                let mut request_builder =
                    client.post(endpoint.clone()).header("Accept", "application/json, text/event-stream").body(json);

                if let Some(id) = session_id.read().await.as_ref() {
                    request_builder = request_builder.header("Mcp-Session-Id", id);
                }

                let response = request_builder.send().await.map_err(StreamableError::from)?;

                if response.status().is_success() {
                    if is_initialize_request {
                        if let Some(header_value) = response.headers().get("Mcp-Session-Id") {
                            if let Ok(id) = header_value.to_str() {
                                *session_id.write().await = Some(id.to_string());
                                debug!("Saved MCP session ID: {}", id);

                                let sse_endpoint = endpoint.clone();
                                let tx_clone = tx.clone();
                                let session_id_value = id.to_string();
                                let client = reqwest::Client::new();

                                tokio::spawn(async move {
                                    let sse_request = client
                                        .get(sse_endpoint)
                                        .header("Accept", "text/event-stream")
                                        .header("Mcp-Session-Id", session_id_value);

                                    match sse_request.send().await {
                                        Ok(sse_response) => {
                                            if sse_response.status().is_success() {
                                                debug!("SSE stream established successfully");

                                                if let Err(e) = setup_sse_client(sse_response, tx_clone).await {
                                                    error!("Error setting up SSE client: {}", e);
                                                }
                                            } else if sse_response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
                                                debug!("Server does not support SSE (405 Method Not Allowed), continuing normally");
                                            } else {
                                                error!("Failed to establish SSE stream: {}", sse_response.status());
                                            }
                                        }
                                        Err(e) => {
                                            error!("Error sending SSE request: {}", e);
                                        }
                                    }
                                });
                            }
                        }
                    }

                    if let Ok(response_body) = response.text().await {
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
                    Err(StreamableError::HttpError(response.status()).into())
                }
            }
        }
    }
}

async fn setup_sse_client(
    response: reqwest::Response,
    on_message: mpsc::Sender<JsonRpcMessage>,
) -> Result<(), StreamableError> {
    let stream = response.bytes_stream();

    tokio::spawn(async move {
        let reader = tokio_util::io::StreamReader::new(
            stream.map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))),
        );

        let mut lines = BufReader::new(reader).lines();

        while let Ok(Some(line)) = lines.next_line().await {
            if line.starts_with("data: ") {
                let data = line.trim_start_matches("data: ");

                if let Ok(json_message) = serde_json::from_str::<JsonRpcMessage>(data) {
                    if let Err(e) = on_message.send(json_message).await {
                        error!("Failed to forward SSE message: {}", e);
                        break;
                    }
                } else {
                    error!("Failed to parse SSE message data as JsonRpcMessage");
                }
            }
        }

        debug!("SSE stream ended");
    });

    Ok(())
}
