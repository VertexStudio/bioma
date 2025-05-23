use super::{SendMessage, Transport, TransportSender};
use crate::client::StreamableConfig as StreamableClientConfig;
use crate::server::{ResponseType, StreamableConfig as StreamableServerConfig};
use crate::transport::Message;
use crate::{ConnectionId, JsonRpcMessage};

use actix_cors::Cors;
use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use anyhow::{Error, Result};
use futures::Stream;
use futures::StreamExt;
use std::collections::HashMap;
use std::future::Future;
use std::net::ToSocketAddrs;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequestId {
    conn_id: ConnectionId,
    jsonrpc_id: jsonrpc_core::Id,
}

impl RequestId {
    fn new(conn_id: ConnectionId, jsonrpc_id: jsonrpc_core::Id) -> Self {
        Self { conn_id, jsonrpc_id }
    }
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{:?}", self.conn_id, self.jsonrpc_id)
    }
}

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
        sse_streams: Arc<RwLock<HashMap<ConnectionId, mpsc::Sender<JsonRpcMessage>>>>,
        pending_requests: Arc<Mutex<HashMap<RequestId, oneshot::Sender<JsonRpcMessage>>>>,
        response_type: ResponseType,
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
        let sse_streams = Arc::new(RwLock::new(HashMap::new()));
        let pending_requests = Arc::new(Mutex::new(HashMap::new()));
        let mode = Arc::new(StreamableMode::Server {
            endpoint: config.endpoint.clone(),
            on_message,
            origins: config.allowed_origins.clone(),
            sse_streams,
            pending_requests,
            response_type: config.response_type.clone(),
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
            StreamableMode::Server { endpoint, on_message, origins, sse_streams, pending_requests, response_type } => {
                let on_message = on_message.clone();
                let shared_sse_streams = sse_streams.clone();
                let shared_pending_requests = pending_requests.clone();

                let app_data = web::Data::new(AppState {
                    on_message,
                    sse_streams: shared_sse_streams,
                    pending_requests: shared_pending_requests,
                    response_type: response_type.clone(),
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
            StreamableMode::Server { sse_streams, pending_requests, .. } => match &message {
                JsonRpcMessage::Response(response) => {
                    let mut pending = pending_requests.lock().await;

                    let response_id = match response {
                        jsonrpc_core::Response::Single(output) => match output {
                            jsonrpc_core::Output::Success(success) => success.id.clone(),
                            jsonrpc_core::Output::Failure(failure) => failure.id.clone(),
                        },
                        jsonrpc_core::Response::Batch(outputs) => {
                            if let Some(first_output) = outputs.first() {
                                match first_output {
                                    jsonrpc_core::Output::Success(success) => success.id.clone(),
                                    jsonrpc_core::Output::Failure(failure) => failure.id.clone(),
                                }
                            } else {
                                jsonrpc_core::Id::Null
                            }
                        }
                    };

                    let request_id_to_remove = pending
                        .keys()
                        .find(|req_id| req_id.conn_id == conn_id && req_id.jsonrpc_id == response_id)
                        .cloned();

                    if let Some(request_id) = request_id_to_remove {
                        if let Some(sender) = pending.remove(&request_id) {
                            sender.send(message).map_err(|_| {
                                StreamableError::ChannelError(format!("Failed to send response to client {}", conn_id))
                            })?;
                            Ok(())
                        } else {
                            Err(StreamableError::ClientNotFound(conn_id).into())
                        }
                    } else {
                        Err(StreamableError::ClientNotFound(conn_id).into())
                    }
                }
                JsonRpcMessage::Request(_) => {
                    let sse_streams_guard = sse_streams.read().await;
                    if let Some(client) = sse_streams_guard.get(&conn_id) {
                        client.send(message).await.map_err(|e| {
                            StreamableError::ChannelError(format!(
                                "Failed to send message to client {}: {}",
                                conn_id, e
                            ))
                        })?;
                        Ok(())
                    } else {
                        Err(StreamableError::ClientNotFound(conn_id).into())
                    }
                }
            },
            StreamableMode::Client { endpoint, tx, session_id, .. } => {
                let json = serde_json::to_string(&message)?;
                let endpoint = endpoint.clone();
                let tx = tx.clone();
                let session_id = session_id.clone();

                let is_initialize_request = match &message {
                    JsonRpcMessage::Request(request) => match request {
                        jsonrpc_core::Request::Single(jsonrpc_core::Call::MethodCall(method_call)) => {
                            method_call.method == "initialize"
                        }
                        _ => false,
                    },
                    _ => false,
                };

                tokio::spawn(async move {
                    let client = reqwest::Client::new();
                    let mut request = client
                        .post(endpoint.clone())
                        .header("Accept", "application/json, text/event-stream")
                        .body(json);

                    if let Some(id) = session_id.read().await.as_ref() {
                        request = request.header("Mcp-Session-Id", id);
                    }

                    match request.send().await {
                        Ok(response) => {
                            if response.status().is_success() {
                                let is_event_stream = response
                                    .headers()
                                    .get(reqwest::header::CONTENT_TYPE)
                                    .and_then(|v| v.to_str().ok())
                                    .map(|s| s.contains("text/event-stream"))
                                    .unwrap_or(false);

                                if is_initialize_request {
                                    let mut session_id_value = None;
                                    if let Some(header_value) = response.headers().get("Mcp-Session-Id") {
                                        if let Ok(id) = header_value.to_str() {
                                            *session_id.write().await = Some(id.to_string());
                                            session_id_value = Some(id.to_string());
                                            debug!("Saved MCP session ID: {}", id);
                                        }
                                    }

                                    let sse_endpoint = endpoint.clone();
                                    let tx_clone = tx.clone();
                                    let client = reqwest::Client::new();

                                    tokio::spawn(async move {
                                        let mut sse_request =
                                            client.get(sse_endpoint).header("Accept", "text/event-stream");

                                        if let Some(id) = session_id_value {
                                            sse_request = sse_request.header("Mcp-Session-Id", id);
                                        }

                                        match sse_request.send().await {
                                            Ok(sse_response) => {
                                                if sse_response.status().is_success() {
                                                    debug!("SSE stream established successfully");

                                                    if let Err(e) = setup_sse_client(sse_response, tx_clone).await {
                                                        error!("Error setting up SSE client: {}", e);
                                                    }
                                                } else if sse_response.status()
                                                    == reqwest::StatusCode::METHOD_NOT_ALLOWED
                                                {
                                                    debug!(
                                                        "Server does not support SSE (405 Method Not Allowed), continuing normally"
                                                    );
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

                                if is_event_stream {
                                    debug!("Received text/event-stream response for request");
                                    let tx_clone = tx.clone();

                                    tokio::spawn(async move {
                                        if let Err(e) = setup_sse_client(response, tx_clone).await {
                                            error!("Error processing text/event-stream response: {}", e);
                                        }
                                    });
                                } else {
                                    if let Ok(response_body) = response.text().await {
                                        if !response_body.is_empty() {
                                            if let Ok(response_message) =
                                                serde_json::from_str::<JsonRpcMessage>(&response_body)
                                            {
                                                if let Err(e) = tx.send(response_message).await {
                                                    error!("Failed to send response through channel: {}", e);
                                                }
                                            }
                                        }
                                    }
                                }
                            } else {
                                error!("HTTP error: {}", response.status());
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
        let mode = self.mode.clone();

        async move {
            match &*mode {
                StreamableMode::Server { sse_streams, pending_requests, .. } => {
                    {
                        let mut streams = sse_streams.write().await;
                        streams.clear();
                        info!("Cleared all SSE streams");
                    }

                    {
                        let mut pending = pending_requests.lock().await;
                        for (conn_id, sender) in pending.drain() {
                            let _ = sender.send(JsonRpcMessage::Response(jsonrpc_core::Response::Single(
                                jsonrpc_core::Output::Failure(jsonrpc_core::Failure {
                                    jsonrpc: Some(jsonrpc_core::Version::V2),
                                    id: jsonrpc_core::Id::Null,
                                    error: jsonrpc_core::Error {
                                        code: jsonrpc_core::ErrorCode::ServerError(-32000),
                                        message: "Server shutting down".to_string(),
                                        data: None,
                                    },
                                }),
                            )));
                            debug!("Completed pending request for connection {} with shutdown error", conn_id);
                        }
                        info!("Completed all pending requests");
                    }

                    info!("Streamable transport server mode closed");
                    Ok(())
                }
                StreamableMode::Client { .. } => {
                    info!("Streamable transport client mode closed");
                    Ok(())
                }
            }
        }
    }
    fn sender(&self) -> TransportSender {
        TransportSender::new_streamable(StreamableTransportSender { mode: self.mode.clone() })
    }
}

struct AppState {
    on_message: mpsc::Sender<Message>,
    sse_streams: Arc<RwLock<HashMap<ConnectionId, mpsc::Sender<JsonRpcMessage>>>>,
    pending_requests: Arc<Mutex<HashMap<RequestId, oneshot::Sender<JsonRpcMessage>>>>,
    response_type: ResponseType,
}

enum SseReceiver {
    Mpsc(mpsc::Receiver<JsonRpcMessage>),
    Oneshot(oneshot::Receiver<JsonRpcMessage>),
}

struct SseStream {
    rx: SseReceiver,
}

impl SseStream {
    fn from_mpsc(rx: mpsc::Receiver<JsonRpcMessage>) -> Self {
        Self { rx: SseReceiver::Mpsc(rx) }
    }

    fn from_oneshot(rx: oneshot::Receiver<JsonRpcMessage>) -> Self {
        Self { rx: SseReceiver::Oneshot(rx) }
    }
}

impl Stream for SseStream {
    type Item = Result<web::Bytes, actix_web::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        let poll_result = match &mut self.rx {
            SseReceiver::Mpsc(rx) => rx.poll_recv(cx),
            SseReceiver::Oneshot(rx) => {
                let pinned = Pin::new(rx);
                match Future::poll(pinned, cx) {
                    Poll::Ready(Ok(msg)) => Poll::Ready(Some(msg)),
                    Poll::Ready(Err(_)) => Poll::Ready(None),
                    Poll::Pending => Poll::Pending,
                }
            }
        };

        match poll_result {
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

#[derive(Debug)]
pub struct Incoming {
    pub conn_id: ConnectionId,

    pub kind: Payload,
}

#[derive(Debug)]
pub enum Payload {
    Notification(JsonRpcMessage),

    Request { id: jsonrpc_core::Id, is_initialize: bool, msg: JsonRpcMessage },

    Response(JsonRpcMessage),
}

#[derive(Debug, thiserror::Error)]
pub enum IncomingError {
    #[error("Invalid JSON-RPC payload: {0}")]
    BadJson(#[from] serde_json::Error),

    #[error("Missing or invalid Mcp-Session-Id header")]
    NoSessionId,
}

impl TryFrom<(HttpRequest, web::Bytes)> for Incoming {
    type Error = IncomingError;

    fn try_from((req, body): (HttpRequest, web::Bytes)) -> Result<Self, Self::Error> {
        use jsonrpc_core::{Call, Request as Rq};

        let msg: JsonRpcMessage = serde_json::from_slice(&body)?;

        let (kind, needs_id) = match &msg {
            JsonRpcMessage::Request(r) => match r {
                Rq::Single(Call::MethodCall(c)) => (
                    Payload::Request { id: c.id.clone(), is_initialize: c.method == "initialize", msg: msg.clone() },
                    c.method != "initialize",
                ),
                Rq::Single(Call::Notification(_)) => (Payload::Notification(msg.clone()), true),
                Rq::Batch(c) if c.iter().all(|x| matches!(x, Call::Notification(_))) => {
                    (Payload::Notification(msg.clone()), true)
                }
                Rq::Batch(calls) => {
                    let id = calls
                        .iter()
                        .find_map(|c| match c {
                            Call::MethodCall(m) => Some(m.id.clone()),
                            _ => None,
                        })
                        .unwrap_or(jsonrpc_core::Id::Null);

                    (Payload::Request { id, is_initialize: false, msg: msg.clone() }, true)
                }
                Rq::Single(Call::Invalid { .. }) => (Payload::Notification(msg.clone()), true),
            },
            JsonRpcMessage::Response(_) => (Payload::Response(msg.clone()), true),
        };

        let conn_id = if let Payload::Request { is_initialize: true, .. } = &kind {
            ConnectionId::new(None)
        } else {
            match req.headers().get("Mcp-Session-Id").and_then(|h| h.to_str().ok()) {
                Some(id) => ConnectionId(id.to_owned()),
                None if needs_id => return Err(IncomingError::NoSessionId),
                None => unreachable!("classifier said we needed session id but header missing"),
            }
        };

        Ok(Self { conn_id, kind })
    }
}

async fn get_handler(req: HttpRequest, app_state: web::Data<AppState>) -> impl Responder {
    if !req.headers().get("Accept").map_or(false, |h| h.to_str().unwrap_or("").contains("text/event-stream")) {
        return HttpResponse::BadRequest().body("Invalid request");
    }

    let session_id = match req.headers().get("Mcp-Session-Id").and_then(|value| value.to_str().ok()).map(String::from) {
        Some(id) => id,
        None => return HttpResponse::BadRequest().body("Missing or invalid Mcp-Session-Id header"),
    };

    let conn_id = ConnectionId(session_id);
    let (tx, rx) = mpsc::channel::<JsonRpcMessage>(32);

    app_state.sse_streams.write().await.insert(conn_id, tx);
    let sse_stream = SseStream::from_mpsc(rx);

    HttpResponse::Ok()
        .content_type("text/event-stream")
        .append_header(("Cache-Control", "no-cache"))
        .append_header(("Connection", "keep-alive"))
        .streaming(sse_stream)
}

async fn post_handler(req: HttpRequest, body: web::Bytes, st: web::Data<AppState>) -> impl Responder {
    let incoming = match Incoming::try_from((req, body)) {
        Ok(i) => i,
        Err(e) => return HttpResponse::BadRequest().body(e.to_string()),
    };

    match incoming.kind {
        Payload::Notification(msg) | Payload::Response(msg) => {
            match st.on_message.send(Message { conn_id: incoming.conn_id, message: msg }).await {
                Ok(_) => HttpResponse::Accepted().finish(),
                Err(e) => {
                    error!("Failed to forward message: {}", e);
                    HttpResponse::InternalServerError().body("Failed to process message")
                }
            }
        }

        Payload::Request { id, is_initialize, msg } => {
            let (tx, rx) = oneshot::channel();
            st.pending_requests.lock().await.insert(RequestId::new(incoming.conn_id.clone(), id), tx);

            if let Err(e) = st.on_message.send(Message { conn_id: incoming.conn_id.clone(), message: msg }).await {
                error!("forward error: {e}");
                return HttpResponse::InternalServerError().finish();
            }

            match st.response_type {
                ResponseType::SSE => {
                    let sse_stream = SseStream::from_oneshot(rx);

                    if is_initialize {
                        HttpResponse::Ok()
                            .content_type("text/event-stream")
                            .append_header(("Cache-Control", "no-cache"))
                            .append_header(("Connection", "keep-alive"))
                            .append_header(("Mcp-Session-Id", incoming.conn_id.0))
                            .streaming(sse_stream)
                    } else {
                        HttpResponse::Ok()
                            .content_type("text/event-stream")
                            .append_header(("Cache-Control", "no-cache"))
                            .append_header(("Connection", "keep-alive"))
                            .streaming(sse_stream)
                    }
                }
                ResponseType::Json => match rx.await {
                    Ok(response) => {
                        if is_initialize {
                            HttpResponse::Ok().append_header(("Mcp-Session-Id", incoming.conn_id.0)).json(response)
                        } else {
                            HttpResponse::Ok().json(response)
                        }
                    }
                    Err(e) => {
                        error!("Failed to receive response: {}", e);
                        HttpResponse::InternalServerError().body("Failed to receive response")
                    }
                },
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
            StreamableMode::Server { sse_streams, pending_requests, .. } => match &message {
                JsonRpcMessage::Response(response) => {
                    let mut pending = pending_requests.lock().await;

                    let response_id = match response {
                        jsonrpc_core::Response::Single(output) => match output {
                            jsonrpc_core::Output::Success(success) => success.id.clone(),
                            jsonrpc_core::Output::Failure(failure) => failure.id.clone(),
                        },
                        jsonrpc_core::Response::Batch(outputs) => {
                            if let Some(first_output) = outputs.first() {
                                match first_output {
                                    jsonrpc_core::Output::Success(success) => success.id.clone(),
                                    jsonrpc_core::Output::Failure(failure) => failure.id.clone(),
                                }
                            } else {
                                jsonrpc_core::Id::Null
                            }
                        }
                    };

                    let request_id_to_remove = pending
                        .keys()
                        .find(|req_id| req_id.conn_id == conn_id && req_id.jsonrpc_id == response_id)
                        .cloned();

                    if let Some(request_id) = request_id_to_remove {
                        if let Some(sender) = pending.remove(&request_id) {
                            sender.send(message).map_err(|_| {
                                StreamableError::ChannelError(format!("Failed to send response to client {}", conn_id))
                            })?;
                            Ok(())
                        } else {
                            Err(StreamableError::ClientNotFound(conn_id).into())
                        }
                    } else {
                        Err(StreamableError::ClientNotFound(conn_id).into())
                    }
                }
                JsonRpcMessage::Request(_) => {
                    let sse_streams_guard = sse_streams.read().await;
                    if let Some(client) = sse_streams_guard.get(&conn_id) {
                        client.send(message).await.map_err(|e| {
                            StreamableError::ChannelError(format!(
                                "Failed to send message to client {}: {}",
                                conn_id, e
                            ))
                        })?;
                        Ok(())
                    } else {
                        Err(StreamableError::ClientNotFound(conn_id).into())
                    }
                }
            },
            StreamableMode::Client { endpoint, tx, session_id, .. } => {
                let json = serde_json::to_string(&message)?;
                let endpoint = endpoint.clone();
                let tx = tx.clone();
                let session_id = session_id.clone();

                let is_initialize_request = match &message {
                    JsonRpcMessage::Request(request) => match request {
                        jsonrpc_core::Request::Single(jsonrpc_core::Call::MethodCall(method_call)) => {
                            method_call.method == "initialize"
                        }
                        _ => false,
                    },
                    _ => false,
                };

                tokio::spawn(async move {
                    let client = reqwest::Client::new();
                    let mut request = client
                        .post(endpoint.clone())
                        .header("Accept", "application/json, text/event-stream")
                        .body(json);

                    if let Some(id) = session_id.read().await.as_ref() {
                        request = request.header("Mcp-Session-Id", id);
                    }

                    match request.send().await {
                        Ok(response) => {
                            if response.status().is_success() {
                                let is_event_stream = response
                                    .headers()
                                    .get(reqwest::header::CONTENT_TYPE)
                                    .and_then(|v| v.to_str().ok())
                                    .map(|s| s.contains("text/event-stream"))
                                    .unwrap_or(false);

                                if is_initialize_request {
                                    let mut session_id_value = None;
                                    if let Some(header_value) = response.headers().get("Mcp-Session-Id") {
                                        if let Ok(id) = header_value.to_str() {
                                            *session_id.write().await = Some(id.to_string());
                                            session_id_value = Some(id.to_string());
                                            debug!("Saved MCP session ID: {}", id);
                                        }
                                    }

                                    let sse_endpoint = endpoint.clone();
                                    let tx_clone = tx.clone();
                                    let client = reqwest::Client::new();

                                    tokio::spawn(async move {
                                        let mut sse_request =
                                            client.get(sse_endpoint).header("Accept", "text/event-stream");

                                        if let Some(id) = session_id_value {
                                            sse_request = sse_request.header("Mcp-Session-Id", id);
                                        }

                                        match sse_request.send().await {
                                            Ok(sse_response) => {
                                                if sse_response.status().is_success() {
                                                    debug!("SSE stream established successfully");

                                                    if let Err(e) = setup_sse_client(sse_response, tx_clone).await {
                                                        error!("Error setting up SSE client: {}", e);
                                                    }
                                                } else if sse_response.status()
                                                    == reqwest::StatusCode::METHOD_NOT_ALLOWED
                                                {
                                                    debug!(
                                                        "Server does not support SSE (405 Method Not Allowed), continuing normally"
                                                    );
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

                                if is_event_stream {
                                    debug!("Received text/event-stream response for request");
                                    let tx_clone = tx.clone();

                                    tokio::spawn(async move {
                                        if let Err(e) = setup_sse_client(response, tx_clone).await {
                                            error!("Error processing text/event-stream response: {}", e);
                                        }
                                    });
                                } else {
                                    if let Ok(response_body) = response.text().await {
                                        if !response_body.is_empty() {
                                            if let Ok(response_message) =
                                                serde_json::from_str::<JsonRpcMessage>(&response_body)
                                            {
                                                if let Err(e) = tx.send(response_message).await {
                                                    error!("Failed to send response through channel: {}", e);
                                                }
                                            }
                                        }
                                    }
                                }
                            } else {
                                error!("HTTP error: {}", response.status());
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
