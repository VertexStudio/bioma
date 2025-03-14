use crate::client::SseConfig as SseClientConfig;
use crate::server::SseConfig as SseServerConfig;
use crate::JsonRpcMessage;

use super::Transport;
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
use tracing::{debug, error, info, warn};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SseMetadata {
    pub client_id: String,
}

/// Transport message type that carries a JsonRpcMessage
#[derive(Debug, Clone)]
pub struct TransportMessage {
    /// The JSON-RPC message being transported
    pub message: JsonRpcMessage,
    /// The event type for SSE
    pub event_type: String,
}

impl TransportMessage {
    /// Create a new transport message with standard "message" event type
    pub fn new(message: JsonRpcMessage) -> Self {
        Self { message, event_type: "message".to_string() }
    }

    /// Create a transport message with a custom event type
    pub fn with_event_type(message: JsonRpcMessage, event_type: impl Into<String>) -> Self {
        Self { message, event_type: event_type.into() }
    }

    /// Format as an SSE event string
    pub fn to_sse_event(&self) -> Result<String> {
        let data = serde_json::to_string(&self.message).context("Failed to serialize JsonRpcMessage")?;
        Ok(format!("event: {}\ndata: {}\n\n", self.event_type, data))
    }
}

/// System message types that don't contain JsonRpcMessage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SystemMessage {
    /// Endpoint information message with connection details
    Endpoint {
        /// URL for sending messages
        url: String,
        /// Client identifier
        client_id: String,
    },
    /// Server shutdown notification
    Shutdown {
        /// Reason for shutdown
        reason: String,
    },
}

impl SystemMessage {
    /// Format as an SSE event string
    pub fn to_sse_event(&self) -> Result<String> {
        let (event_type, data) = match self {
            SystemMessage::Endpoint { .. } => ("endpoint", serde_json::to_string(self)?),
            SystemMessage::Shutdown { .. } => ("shutdown", serde_json::to_string(self)?),
        };

        Ok(format!("event: {}\ndata: {}\n\n", event_type, data))
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

/// Type for sending SSE events to clients - can be either a TransportMessage or SystemMessage
#[derive(Debug)]
pub enum SseEvent {
    /// Transport message containing a JsonRpcMessage
    Transport(TransportMessage),
    /// System message for control events
    System(SystemMessage),
}

impl SseEvent {
    /// Convert an SSE event to a formatted string
    pub fn to_string(&self) -> Result<String> {
        match self {
            SseEvent::Transport(msg) => msg.to_sse_event(),
            SseEvent::System(msg) => msg.to_sse_event(),
        }
    }
}

/// Client registry type for SSE server - maps ClientId to message sender
type ClientRegistry = Arc<Mutex<HashMap<ClientId, mpsc::Sender<SseEvent>>>>;

/// SSE transport operating mode
enum SseMode {
    /// Server mode with connected clients, binding address, and channel capacity
    Server { clients: ClientRegistry, endpoint: String, channel_capacity: usize, on_message: mpsc::Sender<SseMessage> },

    /// Client mode connecting to a server
    Client {
        sse_endpoint: String,
        message_endpoint: Arc<Mutex<Option<String>>>,
        http_client: Client,
        retry_count: usize,
        retry_delay: Duration,
        client_id: Arc<Mutex<Option<String>>>,
        on_message: mpsc::Sender<JsonRpcMessage>,
    },
}

/// Parse an SSE event string into components
#[derive(Debug, Clone)]
pub struct ParsedSseEvent {
    /// The event type
    pub event_type: Option<String>,
    /// The data content
    pub data: Option<String>,
}

impl ParsedSseEvent {
    /// Try to parse the data as a JsonRpcMessage
    pub fn parse_json_rpc(&self) -> Result<Option<JsonRpcMessage>> {
        if let Some(data) = &self.data {
            let message = serde_json::from_str::<JsonRpcMessage>(data)?;
            Ok(Some(message))
        } else {
            Ok(None)
        }
    }

    /// Try to parse the data as a SystemMessage
    pub fn parse_system_message(&self) -> Result<Option<SystemMessage>> {
        if let Some(data) = &self.data {
            let message = serde_json::from_str::<SystemMessage>(data)?;
            Ok(Some(message))
        } else {
            Ok(None)
        }
    }
}

pub struct SseMessage {
    pub message: JsonRpcMessage,
    pub client_id: ClientId,
}

/// Server-Sent Events (SSE) transport implementation
#[derive(Clone)]
pub struct SseTransport {
    mode: Arc<SseMode>,
    #[allow(unused)]
    on_error: mpsc::Sender<Error>,
    #[allow(unused)]
    on_close: mpsc::Sender<()>,
}

impl SseTransport {
    /// Create a new SSE transport in server mode
    pub fn new_server(
        config: SseServerConfig,
        on_message: mpsc::Sender<SseMessage>,
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

    /// Create a new SSE transport in client mode
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
                retry_count: config.retry_count,
                retry_delay: config.retry_delay,
                client_id: Arc::new(Mutex::new(None)),
                on_message,
            }),
            on_error,
            on_close,
        })
    }

    /// Set standard SSE headers on a response
    fn set_sse_headers<T>(response: &mut Response<T>) {
        response.headers_mut().insert(header::CONTENT_TYPE, header::HeaderValue::from_static("text/event-stream"));
        response.headers_mut().insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-cache"));
        response.headers_mut().insert(header::CONNECTION, header::HeaderValue::from_static("keep-alive"));
    }

    /// Parse an SSE event string into a ParsedSseEvent
    pub fn parse_sse_event(event: &str) -> ParsedSseEvent {
        let mut event_type = None;
        let mut event_data = None;

        for line in event.lines() {
            if let Some(data) = line.strip_prefix("data: ") {
                event_data = Some(data.to_string());
            } else if let Some(typ) = line.strip_prefix("event: ") {
                event_type = Some(typ.to_string());
            }
        }

        ParsedSseEvent { event_type, data: event_data }
    }

    /// Helper method to send a message to a specific client
    async fn send_to_client(clients: &ClientRegistry, client_id: &ClientId, event: SseEvent) -> Result<()> {
        let clients_map = clients.lock().await;

        if let Some(tx) = clients_map.get(client_id) {
            if tx.send(event).await.is_err() {
                debug!("Client {} disconnected", client_id.as_ref());
                // We'll handle client removal outside this function
            }
        } else {
            debug!("Client {} not found", client_id.as_ref());
            return Err(SseError::Other(format!("Client {} not found", client_id.as_ref())).into());
        }

        Ok(())
    }

    /// Connect to an SSE endpoint and process events
    async fn connect_to_sse(
        sse_endpoint: &str,
        http_client: &Client,
        message_endpoint: &Arc<Mutex<Option<String>>>,
        client_id: &Arc<Mutex<Option<String>>>,
        on_message: mpsc::Sender<JsonRpcMessage>,
    ) -> Result<()> {
        // Connect to SSE endpoint
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

                // Parse the event using the helper function
                let parsed_event = Self::parse_sse_event(&event);

                match parsed_event.event_type.as_deref() {
                    // Handle endpoint event - get the URL for sending messages and client ID
                    Some("endpoint") => {
                        if let Some(system_message) =
                            parsed_event.parse_system_message().context("Failed to parse system message").ok().flatten()
                        {
                            if let SystemMessage::Endpoint { url, client_id: id } = system_message {
                                // Set the endpoint URL
                                let mut message_endpoint_guard = message_endpoint.lock().await;
                                *message_endpoint_guard = Some(url);

                                // Set the client ID
                                let mut client_id_guard = client_id.lock().await;
                                *client_id_guard = Some(id);

                                debug!("Connection established - endpoint URL and client ID set");
                            }
                        }
                    }
                    // Handle message event - forward to handler
                    Some("message") => {
                        if let Some(json_rpc_message) =
                            parsed_event.parse_json_rpc().context("Failed to parse JSON-RPC message").ok().flatten()
                        {
                            if on_message.send(json_rpc_message).await.is_err() {
                                error!("Failed to forward message - channel closed");
                                return Err(SseError::ChannelError("Message channel closed".to_string()).into());
                            }
                        }
                    }
                    // Handle shutdown event
                    Some("shutdown") => {
                        info!("Received shutdown event from server");
                        return Ok(());
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
    fn start(&mut self) -> impl std::future::Future<Output = Result<JoinHandle<Result<()>>>> {
        let mode = self.mode.clone();

        async move {
            match *mode {
                SseMode::Server { ref clients, ref endpoint, channel_capacity, ref on_message } => {
                    let clients = clients.clone();
                    let on_message = on_message.clone();
                    let endpoint = endpoint.clone();

                    info!("Starting SSE server on {}", endpoint);

                    // Start HTTP server
                    let listener =
                        tokio::net::TcpListener::bind(endpoint.clone()).await.context("Failed to bind to socket")?;

                    // Create a task to handle connections
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

                            // Clone everything needed for the connection handler
                            let clients_clone = clients.clone();
                            let on_message_clone = on_message.clone();
                            let endpoint_clone = endpoint.clone();
                            let capacity = channel_capacity;

                            // Spawn a task to serve the connection
                            tokio::task::spawn(async move {
                                // Create HTTP service to handle SSE connections and message receiving
                                let service = service_fn(move |req: Request<hyper::body::Incoming>| {
                                    let clients = clients_clone.clone();
                                    let on_message = on_message_clone.clone();
                                    let endpoint = endpoint_clone.clone();

                                    async move {
                                        match (req.method(), req.uri().path()) {
                                            // SSE endpoint for clients to connect and receive events
                                            (&Method::GET, "/") => {
                                                debug!("New SSE client connected");

                                                // Create a channel for sending messages to this client
                                                let (client_tx, mut client_rx) = mpsc::channel::<SseEvent>(capacity);
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
                                                    let endpoint_url = format!("http://{}/message", endpoint);
                                                    let endpoint_message = SystemMessage::Endpoint {
                                                        url: endpoint_url,
                                                        client_id: client_id.as_ref().to_string(),
                                                    };

                                                    // Convert to SSE event string
                                                    let endpoint_event = match endpoint_message.to_sse_event() {
                                                        Ok(event) => event,
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
                                                        error!("Failed to send initial endpoint event");
                                                        return;
                                                    }

                                                    // Process incoming events from the client_rx channel
                                                    while let Some(event) = client_rx.recv().await {
                                                        match event.to_string() {
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
                                                let message_str = String::from_utf8_lossy(&bytes).to_string();

                                                debug!("Received client message: {}", message_str);

                                                // Parse to JsonRpcMessage
                                                match serde_json::from_str::<JsonRpcMessage>(&message_str) {
                                                    Ok(json_rpc_message) => {
                                                        if let Some(client_id) = client_id_header {
                                                            // Forward the parsed message
                                                            if on_message
                                                                .send(SseMessage {
                                                                    message: json_rpc_message,
                                                                    client_id,
                                                                })
                                                                .await
                                                                .is_err()
                                                            {
                                                                error!("Failed to forward message - channel closed");
                                                            }
                                                        }
                                                    }
                                                    Err(e) => {
                                                        error!("Failed to parse message: {}", e);
                                                    }
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

                        #[allow(unreachable_code)]
                        Ok(())
                    });

                    Ok(server_handle)
                }
                SseMode::Client {
                    ref sse_endpoint,
                    ref message_endpoint,
                    ref http_client,
                    retry_count,
                    retry_delay,
                    ref client_id,
                    ref on_message,
                } => {
                    let sse_endpoint = sse_endpoint.clone();
                    let message_endpoint = message_endpoint.clone();
                    let http_client = http_client.clone();
                    let client_id = client_id.clone();
                    let on_message = on_message.clone();

                    info!("Starting SSE client, connecting to {}", sse_endpoint);

                    let client_mode_handle = tokio::spawn({
                        async move {
                            let mut attempts = 0;
                            let mut last_error = None;

                            // Implement retry logic
                            while attempts < retry_count {
                                attempts += 1;

                                match Self::connect_to_sse(
                                    &sse_endpoint,
                                    &http_client,
                                    &message_endpoint,
                                    &client_id,
                                    on_message.clone(),
                                )
                                .await
                                {
                                    Ok(_) => return Ok(()),
                                    Err(e) => {
                                        last_error = Some(e);
                                        warn!(
                                            "Connection attempt {}/{} failed, retrying in {:?}",
                                            attempts, retry_count, retry_delay
                                        );
                                        tokio::time::sleep(retry_delay).await;
                                    }
                                }
                            }

                            Err(last_error.unwrap_or_else(|| {
                                SseError::Other("Failed to connect after retries".to_string()).into()
                            }))
                        }
                    });

                    Ok(client_mode_handle)
                }
            }
        }
    }

    fn send(
        &mut self,
        message: JsonRpcMessage,
        metadata: serde_json::Value,
    ) -> impl std::future::Future<Output = Result<()>> {
        let mode = self.mode.clone();

        async move {
            match &*mode {
                SseMode::Server { clients, .. } => {
                    debug!("Server sending [sse] JsonRpcMessage");

                    // Get client_id from metadata

                    // Since we know metadata is SseMetadata for SSE transport
                    if let Some(sse_metadata) = serde_json::from_value::<SseMetadata>(metadata).ok() {
                        let client_id = ClientId(sse_metadata.client_id.clone());

                        // Create TransportMessage and wrap in SseEvent
                        let transport_message = TransportMessage::new(message);
                        let sse_event = SseEvent::Transport(transport_message);

                        // Send event to the specific client
                        Self::send_to_client(clients, &client_id, sse_event).await?;
                    } else {
                        return Err(SseError::Other("Invalid metadata type provided".to_string()).into());
                    }

                    Ok(())
                }
                SseMode::Client { message_endpoint, http_client, client_id, .. } => {
                    debug!("Client sending [sse] JsonRpcMessage");

                    // Get endpoint URL
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

                    // Serialize the message
                    let message_str = serde_json::to_string(&message).context("Failed to serialize JsonRpcMessage")?;

                    // Send HTTP POST request with client_id header
                    let response = http_client
                        .post(&url)
                        .header("Content-Type", "application/json")
                        .header("X-Client-ID", client_id_value)
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

                    // Send a shutdown event to all connected clients
                    for (client_id, tx) in clients_map.drain() {
                        debug!("Sending shutdown event to client {}", client_id.as_ref());

                        // Create shutdown system message
                        let shutdown_message =
                            SystemMessage::Shutdown { reason: "Server is shutting down".to_string() };

                        // Wrap in SseEvent
                        let shutdown_event = SseEvent::System(shutdown_message);

                        // Send the shutdown event to the client
                        if tx.send(shutdown_event).await.is_err() {
                            debug!("Client {} already disconnected", client_id.as_ref());
                        }

                        // The client connection will be closed when tx is dropped
                    }

                    info!("SSE server shutdown completed");
                    Ok(())
                }
                SseMode::Client { sse_endpoint, .. } => {
                    // Client mode: log the shutdown request
                    info!("Closing SSE client connection to {}", sse_endpoint);

                    // The SSE connection will be closed when the task is dropped
                    // No explicit closure is needed

                    Ok(())
                }
            }
        }
    }
}
