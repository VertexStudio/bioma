use crate::client::SseConfig as SseClientConfig;
use crate::server::SseConfig as SseServerConfig;
use crate::transport::Message;
use crate::{ConnectionId, JsonRpcMessage};

use super::{SendMessage, Transport, TransportSender};
use anyhow::{Context, Error, Result};
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{BodyExt, Empty};
use hyper::{body::Frame, header, service::service_fn, Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as HyperServerBuilder;
use reqwest::{Client, ClientBuilder};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, info};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shutdown {
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SseEvent {
    #[serde(rename = "message")]
    Message(JsonRpcMessage),

    #[serde(rename = "endpoint")]
    Endpoint(String),

    #[serde(rename = "shutdown")]
    Shutdown(Shutdown),
}

impl SseEvent {
    const EVENT_TYPE_MESSAGE: &'static str = "message";
    const EVENT_TYPE_ENDPOINT: &'static str = "endpoint";
    const EVENT_TYPE_SHUTDOWN: &'static str = "shutdown";

    pub fn to_sse_string(&self) -> Result<String> {
        let event_type = match self {
            SseEvent::Message(_) => Self::EVENT_TYPE_MESSAGE,
            SseEvent::Endpoint(_) => Self::EVENT_TYPE_ENDPOINT,
            SseEvent::Shutdown(_) => Self::EVENT_TYPE_SHUTDOWN,
        };

        let data = serde_json::to_string(self)?;

        Ok(format!("event: {}\ndata: {}\n\n", event_type, data))
    }

    pub fn from_sse_string(event_str: &str) -> Result<Option<Self>> {
        let mut event_type = None;
        let mut data = None;

        for line in event_str.lines() {
            if let Some(value) = line.strip_prefix("data: ") {
                data = Some(value.to_string());
            } else if let Some(value) = line.strip_prefix("event: ") {
                event_type = Some(value.to_string());
            }
        }

        let event_type = match event_type {
            Some(et) => et,
            None => return Ok(None),
        };

        let data = match data {
            Some(d) => d,
            None => return Ok(None),
        };

        match event_type.as_str() {
            Self::EVENT_TYPE_MESSAGE => {
                let message: JsonRpcMessage = serde_json::from_str(&data)?;
                Ok(Some(SseEvent::Message(message)))
            }
            Self::EVENT_TYPE_ENDPOINT => {
                let endpoint_url = data.trim_matches('"').to_string();
                Ok(Some(SseEvent::Endpoint(endpoint_url)))
            }
            Self::EVENT_TYPE_SHUTDOWN => {
                let shutdown: Shutdown = serde_json::from_str(&data)?;
                Ok(Some(SseEvent::Shutdown(shutdown)))
            }
            _ => Ok(None),
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum SseError {
    #[error("HTTP error: {0}")]
    HttpError(StatusCode),

    #[error("Channel error: {0}")]
    ChannelError(String),

    #[error("Hyper error: {0}")]
    HyperError(#[from] hyper::Error),

    #[error("HTTP builder error: {0}")]
    HttpBuilderError(#[from] hyper::http::Error),

    #[error("Client not found")]
    ClientNotFound,

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("SSE error: {0}")]
    Other(String),
}

type ClientRegistry = Arc<Mutex<HashMap<ConnectionId, mpsc::Sender<SseEvent>>>>;

enum SseMode {
    Server {
        clients: ClientRegistry,
        endpoint: String,
        channel_capacity: usize,
        on_message: mpsc::Sender<Message>,
    },

    Client {
        sse_endpoint: String,
        message_endpoint: Arc<Mutex<Option<String>>>,
        http_client: Client,
        on_message: mpsc::Sender<JsonRpcMessage>,
    },
}

#[derive(Clone)]
pub struct SseTransport {
    mode: Arc<SseMode>,
    #[allow(unused)]
    on_error: mpsc::Sender<Error>,
    #[allow(unused)]
    on_close: mpsc::Sender<()>,
}

impl SseTransport {
    pub fn new_server(
        config: SseServerConfig,
        on_message: mpsc::Sender<Message>,
        on_error: mpsc::Sender<Error>,
        on_close: mpsc::Sender<()>,
    ) -> Self {
        let clients = Arc::new(Mutex::new(HashMap::new()));

        Self {
            mode: Arc::new(SseMode::Server {
                clients,
                endpoint: config.endpoint,
                channel_capacity: config.channel_capacity,
                on_message,
            }),
            on_error,
            on_close,
        }
    }

    pub fn new_client(
        config: &SseClientConfig,
        on_message: mpsc::Sender<JsonRpcMessage>,
        on_error: mpsc::Sender<Error>,
        on_close: mpsc::Sender<()>,
    ) -> Result<Self> {
        let http_client = ClientBuilder::new().build().context("Failed to create HTTP client")?;

        Ok(Self {
            mode: Arc::new(SseMode::Client {
                sse_endpoint: config.endpoint.clone(),
                message_endpoint: Arc::new(Mutex::new(None)),
                http_client,
                on_message,
            }),
            on_error,
            on_close,
        })
    }

    fn set_sse_headers<T>(response: &mut Response<T>) {
        response.headers_mut().insert(header::CONTENT_TYPE, header::HeaderValue::from_static("text/event-stream"));
        response.headers_mut().insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-cache"));
        response.headers_mut().insert(header::CONNECTION, header::HeaderValue::from_static("keep-alive"));
    }

    async fn send_to_client(clients: &ClientRegistry, conn_id: &ConnectionId, event: SseEvent) -> Result<()> {
        let clients_map = clients.lock().await;

        if let Some(tx) = clients_map.get(conn_id) {
            if tx.send(event).await.is_err() {
                debug!("Client {} disconnected", conn_id.to_string());
            }
        } else {
            debug!("Client {} not found", conn_id.to_string());
            return Err(SseError::ClientNotFound.into());
        }

        Ok(())
    }

    async fn connect_to_sse(
        sse_endpoint: &str,
        http_client: &Client,
        message_endpoint: &Arc<Mutex<Option<String>>>,
        on_message: mpsc::Sender<JsonRpcMessage>,
        sse_endpoint_ready_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    ) -> Result<()> {
        let response = http_client
            .get(sse_endpoint)
            .header("Accept", "text/event-stream")
            .send()
            .await
            .context("Failed to connect to SSE endpoint")?;

        if !response.status().is_success() {
            return Err(SseError::HttpError(response.status()).into());
        }

        info!("Connected to SSE endpoint");

        let mut buffer = String::new();
        let mut stream = response.bytes_stream();
        let mut sse_endpoint_ready_sent = false;

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.context("Failed to read SSE chunk")?;
            let chunk_str = String::from_utf8_lossy(&chunk);

            buffer.push_str(&chunk_str);

            while let Some(pos) = buffer.find("\n\n") {
                let event = buffer[..pos + 2].to_string();
                buffer = buffer[pos + 2..].to_string();

                if let Some(sse_event) = SseEvent::from_sse_string(&event)? {
                    match sse_event {
                        SseEvent::Endpoint(endpoint_url) => {
                            let mut message_endpoint_guard = message_endpoint.lock().await;
                            *message_endpoint_guard = Some(endpoint_url);

                            debug!("Connection established - endpoint URL set");

                            if !sse_endpoint_ready_sent {
                                let mut notifier_guard = sse_endpoint_ready_tx.lock().await;
                                if let Some(notifier) = notifier_guard.take() {
                                    let _ = notifier.send(());
                                }
                                sse_endpoint_ready_sent = true;
                            }
                        }

                        SseEvent::Message(json_rpc_message) => {
                            if on_message.send(json_rpc_message).await.is_err() {
                                error!("Failed to forward message - channel closed");
                                return Err(SseError::ChannelError("Message channel closed".to_string()).into());
                            }
                        }

                        SseEvent::Shutdown(shutdown) => {
                            info!("Received shutdown event from server: {}", shutdown.reason);
                            return Ok(());
                        }
                    }
                }
            }
        }

        Err(SseError::Connection("SSE connection closed unexpectedly".to_string()).into())
    }
}

impl Transport for SseTransport {
    fn start(&mut self) -> impl std::future::Future<Output = Result<JoinHandle<Result<()>>>> {
        let mode = self.mode.clone();

        async move {
            match *mode {
                SseMode::Server { ref clients, ref endpoint, channel_capacity, ref on_message } => {
                    let clients = clients.clone();
                    let on_message = on_message.clone();
                    let endpoint = endpoint.clone();

                    info!("Starting SSE server on {}", endpoint);

                    let listener =
                        tokio::net::TcpListener::bind(endpoint.clone()).await.context("Failed to bind to socket")?;

                    let server_handle = tokio::spawn(async move {
                        loop {
                            let (stream, _) = match listener.accept().await {
                                Ok(s) => s,
                                Err(e) => {
                                    error!("Failed to accept connection: {}", e);
                                    continue;
                                }
                            };
                            let io = TokioIo::new(stream);

                            let clients_clone = clients.clone();
                            let on_message_clone = on_message.clone();
                            let endpoint_clone = endpoint.clone();
                            let capacity = channel_capacity;

                            tokio::task::spawn(async move {
                                let service = service_fn(move |req: Request<hyper::body::Incoming>| {
                                    let clients = clients_clone.clone();
                                    let on_message = on_message_clone.clone();
                                    let endpoint = endpoint_clone.clone();

                                    async move {
                                        match (req.method(), req.uri().path()) {
                                            (&Method::GET, "/") => {
                                                debug!("New SSE client connected");

                                                let (client_tx, mut client_rx) = mpsc::channel::<SseEvent>(capacity);
                                                let conn_id = ConnectionId::new(None);

                                                {
                                                    let mut clients_map = clients.lock().await;
                                                    clients_map.insert(conn_id.clone(), client_tx);
                                                }

                                                let (response_tx, response_rx) =
                                                    mpsc::channel::<Result<Frame<Bytes>, std::io::Error>>(capacity);

                                                tokio::spawn(async move {
                                                    let endpoint_url =
                                                        format!("http://{}/sse/{}", endpoint, conn_id.to_string());
                                                    let endpoint_event = SseEvent::Endpoint(endpoint_url);

                                                    let endpoint_event_str = match endpoint_event.to_sse_string() {
                                                        Ok(event) => event,
                                                        Err(err) => {
                                                            error!("Failed to serialize endpoint data: {}", err);
                                                            return;
                                                        }
                                                    };

                                                    if response_tx
                                                        .send(Ok(Frame::data(Bytes::from(endpoint_event_str))))
                                                        .await
                                                        .is_err()
                                                    {
                                                        error!("Failed to send initial endpoint event");
                                                        return;
                                                    }

                                                    while let Some(event) = client_rx.recv().await {
                                                        match event.to_sse_string() {
                                                            Ok(event_str) => {
                                                                if response_tx
                                                                    .send(Ok(Frame::data(Bytes::from(event_str))))
                                                                    .await
                                                                    .is_err()
                                                                {
                                                                    error!(
                                                                        "Client disconnected, stopping event stream"
                                                                    );
                                                                    break;
                                                                }
                                                            }
                                                            Err(e) => {
                                                                error!("Failed to format SSE event: {}", e);
                                                            }
                                                        }
                                                    }
                                                });

                                                let stream = ReceiverStream::new(response_rx);

                                                let body = http_body_util::StreamBody::new(stream);
                                                let mut response = Response::new(http_body_util::Either::Left(body));

                                                Self::set_sse_headers(&mut response);

                                                Ok::<_, SseError>(response)
                                            }

                                            (&Method::POST, path) => {
                                                let conn_id = if let Some(id_str) = path.strip_prefix("/sse/") {
                                                    if let Ok(uuid) = Uuid::parse_str(id_str) {
                                                        ConnectionId(uuid.to_string())
                                                    } else {
                                                        let response = Response::builder()
                                                            .status(StatusCode::BAD_REQUEST)
                                                            .body(http_body_util::Either::Right(Empty::new()))
                                                            .map_err(|e| SseError::HttpBuilderError(e))?;
                                                        return Ok(response);
                                                    }
                                                } else {
                                                    let response = Response::builder()
                                                        .status(StatusCode::NOT_FOUND)
                                                        .body(http_body_util::Either::Right(Empty::new()))
                                                        .map_err(|e| SseError::HttpBuilderError(e))?;
                                                    return Ok(response);
                                                };

                                                let body = req.into_body();
                                                let bytes = body
                                                    .collect()
                                                    .await
                                                    .map_err(|e| SseError::HyperError(e.into()))?
                                                    .to_bytes();
                                                let message_str = String::from_utf8_lossy(&bytes).to_string();

                                                debug!(
                                                    "Received client message from {}: {}",
                                                    conn_id.to_string(),
                                                    message_str
                                                );

                                                match serde_json::from_str::<JsonRpcMessage>(&message_str) {
                                                    Ok(json_rpc_message) => {
                                                        if on_message
                                                            .send(Message { message: json_rpc_message, conn_id })
                                                            .await
                                                            .is_err()
                                                        {
                                                            error!("Failed to forward message - channel closed");
                                                        }
                                                    }
                                                    Err(e) => {
                                                        error!("Failed to parse message: {}", e);
                                                    }
                                                }

                                                let response = Response::builder()
                                                    .status(StatusCode::OK)
                                                    .body(http_body_util::Either::Right(Empty::new()))
                                                    .map_err(|e| SseError::HttpBuilderError(e))?;

                                                Ok::<_, SseError>(response)
                                            }

                                            _ => {
                                                let response = Response::builder()
                                                    .status(StatusCode::NOT_FOUND)
                                                    .body(http_body_util::Either::Right(Empty::new()))
                                                    .map_err(|e| SseError::HttpBuilderError(e))?;

                                                Ok::<_, SseError>(response)
                                            }
                                        }
                                    }
                                });

                                if let Err(err) =
                                    HyperServerBuilder::new(TokioExecutor::new()).serve_connection(io, service).await
                                {
                                    error!("Error serving connection: {:?}", err);
                                }
                            });
                        }

                        #[allow(unreachable_code)]
                        Ok(())
                    });

                    Ok(server_handle)
                }
                SseMode::Client { ref sse_endpoint, ref message_endpoint, ref http_client, ref on_message } => {
                    let sse_endpoint = sse_endpoint.clone();
                    let message_endpoint = message_endpoint.clone();
                    let http_client = http_client.clone();
                    let on_message = on_message.clone();

                    info!("Starting SSE client, connecting to {}", sse_endpoint);

                    let (sse_endpoint_ready_tx, sse_endpoint_ready_rx) = tokio::sync::oneshot::channel();

                    let sse_endpoint_ready_tx = Arc::new(Mutex::new(Some(sse_endpoint_ready_tx)));

                    let client_handle = tokio::spawn({
                        async move {
                            match Self::connect_to_sse(
                                &sse_endpoint,
                                &http_client,
                                &message_endpoint,
                                on_message.clone(),
                                sse_endpoint_ready_tx,
                            )
                            .await
                            {
                                Ok(_) => Ok(()),
                                Err(e) => {
                                    error!("Failed to connect to SSE endpoint: {}", e);
                                    Err(e)
                                }
                            }
                        }
                    });

                    let timeout = tokio::time::timeout(Duration::from_secs(30), sse_endpoint_ready_rx).await;

                    match timeout {
                        Ok(Ok(_)) => {
                            debug!("Endpoint URL set, client connection ready");
                            Ok(client_handle)
                        }
                        Ok(Err(_)) => {
                            Err(SseError::Connection("Endpoint notification channel closed unexpectedly".to_string())
                                .into())
                        }
                        Err(_) => {
                            Err(SseError::Connection("Timed out waiting for endpoint to be set".to_string()).into())
                        }
                    }
                }
            }
        }
    }

    fn send(
        &mut self,
        message: JsonRpcMessage,
        conn_id: ConnectionId,
    ) -> impl std::future::Future<Output = Result<()>> {
        let mode = self.mode.clone();

        async move {
            match &*mode {
                SseMode::Server { clients, .. } => {
                    debug!("Server sending [sse] JsonRpcMessage");

                    let sse_event = SseEvent::Message(message);

                    Self::send_to_client(clients, &conn_id, sse_event).await?;

                    Ok(())
                }
                SseMode::Client { message_endpoint, http_client, .. } => {
                    debug!("Client sending [sse] JsonRpcMessage");

                    let url = {
                        let message_endpoint_guard = message_endpoint.lock().await;
                        match &*message_endpoint_guard {
                            Some(url) => url.clone(),
                            None => {
                                return Err(SseError::Other(
                                    "No endpoint URL available yet. Wait for the SSE connection to establish."
                                        .to_string(),
                                )
                                .into());
                            }
                        }
                    };

                    let message_str = serde_json::to_string(&message).context("Failed to serialize JsonRpcMessage")?;

                    let response = http_client
                        .post(&url)
                        .header("Content-Type", "application/json")
                        .body(message_str)
                        .send()
                        .await
                        .context("Failed to send message")?;

                    if !response.status().is_success() {
                        return Err(SseError::HttpError(response.status()).into());
                    }

                    debug!("Message sent successfully");

                    Ok(())
                }
            }
        }
    }

    fn close(&mut self) -> impl std::future::Future<Output = Result<()>> {
        let mode = self.mode.clone();

        async move {
            match &*mode {
                SseMode::Server { clients, .. } => {
                    info!("Initiating SSE server shutdown");

                    let mut clients_map = clients.lock().await;

                    for (conn_id, tx) in clients_map.drain() {
                        debug!("Sending shutdown event to client {}", conn_id.to_string());

                        let shutdown_event =
                            SseEvent::Shutdown(Shutdown { reason: "Server is shutting down".to_string() });

                        if tx.send(shutdown_event).await.is_err() {
                            debug!("Client {} already disconnected", conn_id.to_string());
                        }
                    }

                    info!("SSE server shutdown completed");
                    Ok(())
                }
                SseMode::Client { sse_endpoint, .. } => {
                    info!("Closing SSE client connection to {}", sse_endpoint);

                    Ok(())
                }
            }
        }
    }

    fn sender(&self) -> TransportSender {
        TransportSender::new_sse(SseTransportSender { mode: self.mode.clone() })
    }
}

#[derive(Clone)]
pub struct SseTransportSender {
    mode: Arc<SseMode>,
}

impl SendMessage for SseTransportSender {
    async fn send(&self, message: JsonRpcMessage, conn_id: ConnectionId) -> Result<()> {
        match &*self.mode {
            SseMode::Server { clients, .. } => {
                let event = SseEvent::Message(message);
                SseTransport::send_to_client(clients, &conn_id, event).await
            }
            SseMode::Client { message_endpoint, http_client, .. } => {
                let endpoint = message_endpoint.lock().await.clone();

                if let Some(endpoint) = endpoint {
                    let json = serde_json::to_string(&message).context("Failed to serialize message")?;

                    let res = http_client
                        .post(&endpoint)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(json)
                        .send()
                        .await
                        .context("Failed to send message")?;

                    if !res.status().is_success() {
                        return Err(Error::msg(format!("HTTP error: {}", res.status())));
                    }
                } else {
                    return Err(Error::msg("No message endpoint available"));
                }

                Ok(())
            }
        }
    }
}
