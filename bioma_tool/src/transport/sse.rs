use super::Transport;
use anyhow::{Context, Result};
use bon::Builder;
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{BodyExt, Empty};
use hyper::{body::Frame, header, service::service_fn, Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as HyperServerBuilder;
use reqwest::{Client, ClientBuilder};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, info};
use url::Url;
use uuid::Uuid;

/// Unique identifier for SSE clients
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClientId(String);

impl ClientId {
    /// Create a new unique client ID
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl AsRef<str> for ClientId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Transport message type
#[derive(Debug, Clone)]
pub struct TransportMessage(String);

impl TransportMessage {
    /// Create a new transport message
    pub fn new(content: String) -> Self {
        Self(content)
    }

    /// Consume the message and return the inner string
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl AsRef<str> for TransportMessage {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// SSE-specific error types
#[derive(Debug, thiserror::Error)]
pub enum SseError {
    #[error("Failed to establish connection: {0}")]
    ConnectionError(#[from] reqwest::Error),

    #[error("Failed to parse URL: {0}")]
    UrlParseError(#[from] url::ParseError),

    #[error("HTTP error: {0}")]
    HttpError(StatusCode),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("JSON serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Channel error: {0}")]
    ChannelError(String),

    #[error("Hyper error: {0}")]
    HyperError(#[from] hyper::Error),

    #[error("HTTP builder error: {0}")]
    HttpBuilderError(#[from] hyper::http::Error),

    #[error("SSE error: {0}")]
    Other(String),
}

fn default_server_url() -> SocketAddr {
    "127.0.0.1:8080".parse().unwrap()
}

/// Server configuration with builder pattern
#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
pub struct SseServerConfig {
    #[builder(default = default_server_url())]
    pub url: SocketAddr,
    #[builder(default = default_channel_capacity())]
    pub channel_capacity: usize,
    #[builder(default = default_keep_alive())]
    pub keep_alive: bool,
}

fn default_channel_capacity() -> usize {
    32
}

fn default_keep_alive() -> bool {
    true
}

impl Default for SseServerConfig {
    fn default() -> Self {
        Self::builder().build()
    }
}

/// Client configuration with builder pattern
#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
pub struct SseClientConfig {
    #[builder(default = default_server_url())]
    pub server_url: SocketAddr,
    #[builder(default = default_retry_count())]
    pub retry_count: usize,
    #[builder(default = default_retry_delay())]
    pub retry_delay: Duration,
}

fn default_retry_count() -> usize {
    3
}

fn default_retry_delay() -> Duration {
    Duration::from_secs(5)
}

impl Default for SseClientConfig {
    fn default() -> Self {
        Self::builder().build()
    }
}

/// Client registry type for SSE server - maps ClientId to message sender
type ClientRegistry = Arc<Mutex<HashMap<ClientId, mpsc::Sender<TransportMessage>>>>;

/// Request registry type - maps JSONRPC message IDs to ClientId
type RequestRegistry = Arc<Mutex<HashMap<String, ClientId>>>;

/// SSE transport operating mode
enum SseMode {
    /// Server mode with connected clients, request registry, and binding address
    Server { clients: ClientRegistry, requests: RequestRegistry, url: SocketAddr, channel_capacity: usize },

    /// Client mode connecting to a server
    Client {
        sse_url: Url,
        endpoint_url: Arc<Mutex<Option<String>>>,
        http_client: Client,
        retry_count: usize,
        retry_delay: Duration,
        client_id: Arc<Mutex<Option<String>>>,
    },
}

/// Format data as an SSE event with a specific type
fn format_sse_event(event_type: &str, data: &str) -> String {
    format!("event: {}\ndata: {}\n\n", event_type, data)
}

/// Server-Sent Events (SSE) transport implementation
#[derive(Clone)]
pub struct SseTransport {
    mode: Arc<SseMode>,
}

impl SseTransport {
    /// Create a new SSE transport in server mode
    pub fn new_server(config: SseServerConfig) -> Self {
        let clients = Arc::new(Mutex::new(HashMap::new()));
        let requests = Arc::new(Mutex::new(HashMap::new()));

        Self {
            mode: Arc::new(SseMode::Server {
                clients,
                requests,
                url: config.url,
                channel_capacity: config.channel_capacity,
            }),
        }
    }

    /// Create a new SSE transport in client mode
    pub fn new_client(config: SseClientConfig) -> Result<Self> {
        let http_client = ClientBuilder::new().build().context("Failed to create HTTP client")?;
        let sse_url = Url::parse(&format!("http://{}", config.server_url)).context("Failed to parse server URL")?;

        Ok(Self {
            mode: Arc::new(SseMode::Client {
                sse_url,
                endpoint_url: Arc::new(Mutex::new(None)),
                http_client,
                retry_count: config.retry_count,
                retry_delay: config.retry_delay,
                client_id: Arc::new(Mutex::new(None)),
            }),
        })
    }

    /// Set standard SSE headers on a response
    fn set_sse_headers<T>(response: &mut Response<T>) {
        response.headers_mut().insert(header::CONTENT_TYPE, header::HeaderValue::from_static("text/event-stream"));
        response.headers_mut().insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-cache"));
        response.headers_mut().insert(header::CONNECTION, header::HeaderValue::from_static("keep-alive"));
    }

    /// Parse an SSE event string into (type, data) components
    fn parse_sse_event(event: &str) -> (Option<String>, Option<String>) {
        let mut event_type = None;
        let mut event_data = None;

        for line in event.lines() {
            if let Some(data) = line.strip_prefix("data: ") {
                event_data = Some(data.to_string());
            } else if let Some(typ) = line.strip_prefix("event: ") {
                event_type = Some(typ.to_string());
            }
        }

        (event_type, event_data)
    }

    /// Parse JSON-RPC ID from a message
    fn extract_jsonrpc_id(message: &str) -> Option<String> {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(message) {
            if let Some(id) = json.get("id") {
                if let Some(id_str) = id.as_str() {
                    return Some(id_str.to_string());
                } else if let Some(id_num) = id.as_i64() {
                    return Some(id_num.to_string());
                }
            }
        }
        None
    }

    /// Helper method to send a message to a specific client
    async fn send_to_client(clients: &ClientRegistry, client_id: &ClientId, message: TransportMessage) -> Result<()> {
        let sse_message = format_sse_event("message", message.as_ref());
        let clients_map = clients.lock().await;

        if let Some(tx) = clients_map.get(client_id) {
            if tx.send(TransportMessage::new(sse_message)).await.is_err() {
                debug!("Client {} disconnected", client_id.as_ref());
                // We'll handle client removal outside this function
            }
        } else {
            debug!("Client {} not found", client_id.as_ref());
        }

        Ok(())
    }

    /// Connect to an SSE endpoint and process events
    async fn connect_to_sse(
        sse_url: &Url,
        http_client: &Client,
        endpoint_url: &Arc<Mutex<Option<String>>>,
        client_id: &Arc<Mutex<Option<String>>>,
        request_tx: mpsc::Sender<String>,
    ) -> Result<()> {
        // Connect to SSE endpoint
        let response = http_client
            .get(sse_url.to_string())
            .header("Accept", "text/event-stream")
            .send()
            .await
            .context("Failed to connect to SSE endpoint")?;

        if !response.status().is_success() {
            return Err(SseError::HttpError(response.status()).into());
        }

        info!("Connected to SSE endpoint");

        // Process SSE events from stream
        let mut buffer = String::new();
        let mut stream = response.bytes_stream();

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.context("Failed to read SSE chunk")?;
            let chunk_str = String::from_utf8_lossy(&chunk);

            buffer.push_str(&chunk_str);

            // Process complete events (double newline is the delimiter)
            while let Some(pos) = buffer.find("\n\n") {
                let event = buffer[..pos + 2].to_string();
                buffer = buffer[pos + 2..].to_string();

                // Extract event data using the helper function
                let (event_type, event_data) = Self::parse_sse_event(&event);

                match (event_type, event_data) {
                    // Handle endpoint event - get the URL for sending messages and client ID
                    (Some(typ), Some(data)) if typ == "endpoint" => {
                        debug!("Received endpoint event: {}", data);

                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                            if let Some(url) = json.get("url").and_then(|v| v.as_str()) {
                                let mut endpoint_guard = endpoint_url.lock().await;
                                *endpoint_guard = Some(url.to_string());
                                debug!("Endpoint URL set to: {}", url);
                            }

                            // Extract client_id from endpoint event
                            if let Some(id) = json.get("client_id").and_then(|v| v.as_str()) {
                                let mut client_id_guard = client_id.lock().await;
                                *client_id_guard = Some(id.to_string());
                                debug!("Client ID set to: {}", id);
                            }
                        }
                    }
                    // Handle message event - forward to handler
                    (Some(typ), Some(data)) if typ == "message" => {
                        debug!("Received message event: {}", data);

                        if request_tx.send(data).await.is_err() {
                            error!("Failed to forward message");
                            return Err(SseError::ChannelError("Message channel closed".to_string()).into());
                        }
                    }
                    // Ignore other event types
                    _ => {}
                }
            }
        }

        Err(SseError::Other("SSE connection closed unexpectedly".to_string()).into())
    }
}

impl Transport for SseTransport {
    async fn start(&mut self, request_tx: mpsc::Sender<String>) -> Result<()> {
        match &*self.mode {
            SseMode::Server { clients, requests, url, channel_capacity } => {
                info!("Starting SSE server on {}", url);

                // Start HTTP server
                let listener = tokio::net::TcpListener::bind(*url).await.context("Failed to bind to socket")?;

                // Process incoming connections
                loop {
                    let (stream, _) = listener.accept().await.context("Failed to accept connection")?;
                    let io = TokioIo::new(stream);

                    // Clone everything needed for the connection handler
                    let clients_clone = clients.clone();
                    let requests_clone = requests.clone();
                    let request_tx_clone = request_tx.clone();
                    let url_clone = *url;
                    let capacity = *channel_capacity;

                    // Spawn a task to serve the connection
                    tokio::task::spawn(async move {
                        // Create HTTP service to handle SSE connections and message receiving
                        let service = service_fn(move |req: Request<hyper::body::Incoming>| {
                            let clients = clients_clone.clone();
                            let requests = requests_clone.clone();
                            let request_tx = request_tx_clone.clone();
                            let url = url_clone;

                            async move {
                                match (req.method(), req.uri().path()) {
                                    // SSE endpoint for clients to connect and receive events
                                    (&Method::GET, "/") => {
                                        debug!("New SSE client connected");

                                        // Create a channel for sending messages to this client
                                        let (client_tx, mut client_rx) = mpsc::channel::<TransportMessage>(capacity);
                                        let client_id = ClientId::new();

                                        // Register client
                                        {
                                            let mut clients_map = clients.lock().await;
                                            clients_map.insert(client_id.clone(), client_tx);
                                        }

                                        // Create a new channel for the streaming response
                                        let (response_tx, response_rx) =
                                            mpsc::channel::<Result<Frame<Bytes>, std::io::Error>>(capacity);

                                        // Spawn a task to handle sending SSE events to the client
                                        tokio::spawn(async move {
                                            // Send initial endpoint event with client_id
                                            let endpoint_url = format!("http://{}/message", url);
                                            let endpoint_data = serde_json::json!({
                                                "url": endpoint_url,
                                                "client_id": client_id.as_ref()
                                            });

                                            let endpoint_event = match serde_json::to_string(&endpoint_data) {
                                                Ok(json) => format_sse_event("endpoint", &json),
                                                Err(err) => {
                                                    error!("Failed to serialize endpoint data: {}", err);
                                                    return;
                                                }
                                            };

                                            // Send the initial event to the client via the response channel
                                            if response_tx
                                                .send(Ok(Frame::data(Bytes::from(endpoint_event))))
                                                .await
                                                .is_err()
                                            {
                                                error!("Failed to send initial endpoint event to response stream");
                                                return;
                                            }

                                            // Process incoming events from the client_rx channel
                                            while let Some(event) = client_rx.recv().await {
                                                if response_tx
                                                    .send(Ok(Frame::data(Bytes::from(event.into_inner()))))
                                                    .await
                                                    .is_err()
                                                {
                                                    error!("Client disconnected, stopping event stream");
                                                    break;
                                                }
                                            }
                                        });

                                        // Create a stream from the receiver
                                        let stream = ReceiverStream::new(response_rx);

                                        // Build SSE response with the stream
                                        let body = http_body_util::StreamBody::new(stream);
                                        let mut response = Response::new(http_body_util::Either::Left(body));

                                        // Set SSE headers
                                        Self::set_sse_headers(&mut response);

                                        Ok::<_, SseError>(response)
                                    }
                                    // Message endpoint for receiving client messages
                                    (&Method::POST, "/message") => {
                                        // Extract client ID from headers
                                        let client_id_header = req
                                            .headers()
                                            .get("X-Client-ID")
                                            .and_then(|h| h.to_str().ok())
                                            .map(|s| ClientId(s.to_string()));

                                        // Get message from request body
                                        let body = req.into_body();
                                        let bytes = body
                                            .collect()
                                            .await
                                            .map_err(|e| SseError::HyperError(e.into()))?
                                            .to_bytes();
                                        let message = String::from_utf8_lossy(&bytes).to_string();

                                        debug!("Received client message: {}", message);

                                        // Extract JSON-RPC ID from message
                                        if let Some(jsonrpc_id) = Self::extract_jsonrpc_id(&message) {
                                            // Store mapping of jsonrpc_id to client_id if we have a client_id
                                            if let Some(client_id) = client_id_header {
                                                let mut requests_map = requests.lock().await;
                                                requests_map.insert(jsonrpc_id, client_id);
                                            }
                                        }

                                        // Forward to message handler
                                        if request_tx.send(message).await.is_err() {
                                            error!("Failed to forward message");
                                        }

                                        // Return OK response
                                        let response = Response::builder()
                                            .status(StatusCode::OK)
                                            .body(http_body_util::Either::Right(Empty::new()))
                                            .map_err(|e| SseError::HttpBuilderError(e))?;

                                        Ok::<_, SseError>(response)
                                    }
                                    // Any other endpoint
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
            }
            SseMode::Client { sse_url, endpoint_url, http_client, retry_count, retry_delay, client_id } => {
                info!("Starting SSE client, connecting to {}", sse_url);

                let mut attempts = 0;
                let mut last_error = None;

                // Implement retry logic
                while attempts < *retry_count {
                    attempts += 1;

                    match Self::connect_to_sse(sse_url, http_client, endpoint_url, client_id, request_tx.clone()).await
                    {
                        Ok(_) => return Ok(()),
                        Err(e) => {
                            last_error = Some(e);
                            info!(
                                "Connection attempt {}/{} failed, retrying in {:?}",
                                attempts, retry_count, retry_delay
                            );
                            tokio::time::sleep(*retry_delay).await;
                        }
                    }
                }

                Err(last_error.unwrap_or_else(|| SseError::Other("Failed to connect after retries".to_string()).into()))
            }
        }
    }

    async fn send(&mut self, message: String) -> Result<()> {
        match &*self.mode {
            SseMode::Server { clients, requests, .. } => {
                debug!("Server sending [sse]: {}", message);

                // Extract JSON-RPC ID from message
                if let Some(jsonrpc_id) = Self::extract_jsonrpc_id(&message) {
                    // Find the client_id for this JSON-RPC ID
                    let client_id = {
                        let requests_map = requests.lock().await;
                        requests_map.get(&jsonrpc_id).cloned()
                    };

                    if let Some(client_id) = client_id {
                        // Send message to the specific client
                        Self::send_to_client(clients, &client_id, TransportMessage::new(message)).await?;

                        // Clean up the request map entry after sending
                        let mut requests_map = requests.lock().await;
                        requests_map.remove(&jsonrpc_id);
                    } else {
                        return Err(SseError::Other(format!("No client found for JSON-RPC ID: {}", jsonrpc_id)).into());
                    }
                } else {
                    return Err(SseError::Other("Message has no JSON-RPC ID, cannot route properly".to_string()).into());
                }

                Ok(())
            }
            SseMode::Client { endpoint_url, http_client, client_id, .. } => {
                debug!("Client sending [sse]: {}", message);

                // Get endpoint URL
                let url = {
                    let endpoint_guard = endpoint_url.lock().await;
                    match &*endpoint_guard {
                        Some(url) => url.clone(),
                        None => {
                            return Err(SseError::Other(
                                "No endpoint URL available yet. Wait for the SSE connection to establish.".to_string(),
                            )
                            .into());
                        }
                    }
                };

                // Get client_id
                let client_id_value = {
                    let client_id_guard = client_id.lock().await;
                    match &*client_id_guard {
                        Some(id) => id.clone(),
                        None => {
                            return Err(SseError::Other(
                                "No client ID available yet. Wait for the SSE connection to establish.".to_string(),
                            )
                            .into());
                        }
                    }
                };

                // Send HTTP POST request with client_id header
                let response = http_client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .header("X-Client-ID", client_id_value)
                    .body(message)
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
