use crate::client::SseConfig as SseClientConfig;
use crate::server::SseConfig as SseServerConfig;
use crate::{ClientId, JsonRpcMessage};

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
use tracing::{debug, error, info};
use uuid::Uuid;

/// Metadata for SSE transport
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SseMetadata {
    pub client_id: ClientId,
}

/// Shutdown event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shutdown {
    /// Reason for shutdown
    pub reason: String,
}

/// SSE event type that handles both transport and system messages
///
/// event: message
/// data: {"jsonrpc": "2.0", ...}
///
/// event: endpoint
/// data: "http://..."
///
/// event: shutdown
/// data: "Server is shutting down"
///
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SseEvent {
    /// Regular JSON-RPC message event
    #[serde(rename = "message")]
    Message(JsonRpcMessage),

    /// Endpoint information for client connection
    #[serde(rename = "endpoint")]
    Endpoint(String),

    /// Server shutdown notification
    #[serde(rename = "shutdown")]
    Shutdown(Shutdown),
}

impl SseEvent {
    const EVENT_TYPE_MESSAGE: &'static str = "message";
    const EVENT_TYPE_ENDPOINT: &'static str = "endpoint";
    const EVENT_TYPE_SHUTDOWN: &'static str = "shutdown";

    /// Format the event as an SSE text format
    pub fn to_sse_string(&self) -> Result<String> {
        // Determine event type from variant
        let event_type = match self {
            SseEvent::Message(_) => Self::EVENT_TYPE_MESSAGE,
            SseEvent::Endpoint(_) => Self::EVENT_TYPE_ENDPOINT,
            SseEvent::Shutdown(_) => Self::EVENT_TYPE_SHUTDOWN,
        };

        // Serialize the data portion
        let data = serde_json::to_string(self)?;

        // Format as SSE event
        Ok(format!("event: {}\ndata: {}\n\n", event_type, data))
    }

    /// Parse an SSE event string into an SseEvent
    pub fn from_sse_string(event_str: &str) -> Result<Option<Self>> {
        let mut event_type = None;
        let mut data = None;

        // Parse the SSE format
        for line in event_str.lines() {
            if let Some(value) = line.strip_prefix("data: ") {
                data = Some(value.to_string());
            } else if let Some(value) = line.strip_prefix("event: ") {
                event_type = Some(value.to_string());
            }
        }

        // If we don't have both event type and data, return None
        let event_type = match event_type {
            Some(et) => et,
            None => return Ok(None),
        };

        let data = match data {
            Some(d) => d,
            None => return Ok(None),
        };

        // Parse the data based on event type
        match event_type.as_str() {
            Self::EVENT_TYPE_MESSAGE => {
                let message: JsonRpcMessage = serde_json::from_str(&data)?;
                Ok(Some(SseEvent::Message(message)))
            }
            Self::EVENT_TYPE_ENDPOINT => {
                // Endpoint is just a string (URL)
                let endpoint_url = data.trim_matches('"').to_string();
                Ok(Some(SseEvent::Endpoint(endpoint_url)))
            }
            Self::EVENT_TYPE_SHUTDOWN => {
                let shutdown: Shutdown = serde_json::from_str(&data)?;
                Ok(Some(SseEvent::Shutdown(shutdown)))
            }
            _ => Ok(None), // Unknown event type
        }
    }
}

/// Message received from a client with associated client ID
pub struct SseMessage {
    pub message: JsonRpcMessage,
    pub client_id: ClientId,
}

/// SSE-specific error types
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
        on_message: mpsc::Sender<JsonRpcMessage>,
    },
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

    /// Helper method to send a message to a specific client
    async fn send_to_client(clients: &ClientRegistry, client_id: &ClientId, event: SseEvent) -> Result<()> {
        let clients_map = clients.lock().await;

        if let Some(tx) = clients_map.get(client_id) {
            if tx.send(event).await.is_err() {
                debug!("Client {} disconnected", client_id.to_string());
                // We'll handle client removal outside this function
            }
        } else {
            debug!("Client {} not found", client_id.to_string());
            return Err(SseError::ClientNotFound.into());
        }

        Ok(())
    }

    /// Connect to an SSE endpoint and process events
    async fn connect_to_sse(
        sse_endpoint: &str,
        http_client: &Client,
        message_endpoint: &Arc<Mutex<Option<String>>>,
        on_message: mpsc::Sender<JsonRpcMessage>,
        endpoint_notifier: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
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
        let mut endpoint_notified = false;

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.context("Failed to read SSE chunk")?;
            let chunk_str = String::from_utf8_lossy(&chunk);

            buffer.push_str(&chunk_str);

            // Process complete events (double newline is the delimiter)
            while let Some(pos) = buffer.find("\n\n") {
                let event = buffer[..pos + 2].to_string();
                buffer = buffer[pos + 2..].to_string();

                // Parse the event
                if let Some(sse_event) = SseEvent::from_sse_string(&event)? {
                    match sse_event {
                        // Handle endpoint event
                        SseEvent::Endpoint(endpoint_url) => {
                            // Set the endpoint URL
                            let mut message_endpoint_guard = message_endpoint.lock().await;
                            *message_endpoint_guard = Some(endpoint_url);

                            debug!("Connection established - endpoint URL set");

                            // Notify that endpoint is set
                            if !endpoint_notified {
                                let mut notifier_guard = endpoint_notifier.lock().await;
                                if let Some(notifier) = notifier_guard.take() {
                                    // We don't care if receiver is dropped
                                    let _ = notifier.send(());
                                }
                                endpoint_notified = true;
                            }
                        }
                        // Handle message event - forward to handler
                        SseEvent::Message(json_rpc_message) => {
                            if on_message.send(json_rpc_message).await.is_err() {
                                error!("Failed to forward message - channel closed");
                                return Err(SseError::ChannelError("Message channel closed".to_string()).into());
                            }
                        }
                        // Handle shutdown event
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
                                                    // Send initial endpoint event with path that includes client_id
                                                    let endpoint_url =
                                                        format!("http://{}/sse/{}", endpoint, client_id.to_string());
                                                    let endpoint_event = SseEvent::Endpoint(endpoint_url);

                                                    // Convert to SSE event string
                                                    let endpoint_event_str = match endpoint_event.to_sse_string() {
                                                        Ok(event) => event,
                                                        Err(err) => {
                                                            error!("Failed to serialize endpoint data: {}", err);
                                                            return;
                                                        }
                                                    };

                                                    // Send the initial event to the client via the response channel
                                                    if response_tx
                                                        .send(Ok(Frame::data(Bytes::from(endpoint_event_str))))
                                                        .await
                                                        .is_err()
                                                    {
                                                        error!("Failed to send initial endpoint event");
                                                        return;
                                                    }

                                                    // Process incoming events from the client_rx channel
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
                                            (&Method::POST, path) => {
                                                // Extract client ID from path
                                                let client_id = if let Some(id_str) = path.strip_prefix("/sse/") {
                                                    if let Ok(uuid) = Uuid::parse_str(id_str) {
                                                        ClientId(uuid)
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

                                                // Get message from request body
                                                let body = req.into_body();
                                                let bytes = body
                                                    .collect()
                                                    .await
                                                    .map_err(|e| SseError::HyperError(e.into()))?
                                                    .to_bytes();
                                                let message_str = String::from_utf8_lossy(&bytes).to_string();

                                                debug!(
                                                    "Received client message from {}: {}",
                                                    client_id.to_string(),
                                                    message_str
                                                );

                                                // Parse to JsonRpcMessage
                                                match serde_json::from_str::<JsonRpcMessage>(&message_str) {
                                                    Ok(json_rpc_message) => {
                                                        // Forward the parsed message
                                                        if on_message
                                                            .send(SseMessage { message: json_rpc_message, client_id })
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
                SseMode::Client { ref sse_endpoint, ref message_endpoint, ref http_client, ref on_message } => {
                    let sse_endpoint = sse_endpoint.clone();
                    let message_endpoint = message_endpoint.clone();
                    let http_client = http_client.clone();
                    let on_message = on_message.clone();

                    info!("Starting SSE client, connecting to {}", sse_endpoint);

                    // We'll create a notification channel to directly communicate when endpoint is set
                    let (endpoint_set_tx, endpoint_set_rx) = tokio::sync::oneshot::channel();

                    // Store this in a wrapper we can send to our connection function
                    let endpoint_notifier = Arc::new(Mutex::new(Some(endpoint_set_tx)));

                    // Create the client connection task
                    let client_handle = tokio::spawn({
                        let endpoint_notifier = endpoint_notifier.clone();

                        async move {
                            match Self::connect_to_sse(
                                &sse_endpoint,
                                &http_client,
                                &message_endpoint,
                                on_message.clone(),
                                endpoint_notifier.clone(),
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

                    // Wait for the endpoint to be set (with timeout)
                    let timeout = tokio::time::timeout(Duration::from_secs(30), endpoint_set_rx).await;

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
        metadata: serde_json::Value,
    ) -> impl std::future::Future<Output = Result<()>> {
        let mode = self.mode.clone();

        async move {
            match &*mode {
                SseMode::Server { clients, .. } => {
                    debug!("Server sending [sse] JsonRpcMessage");

                    // Get client_id from metadata
                    // Using SseMetadata instead of direct ClientId deserialization
                    if let Some(sse_metadata) = serde_json::from_value::<SseMetadata>(metadata).ok() {
                        let client_id = sse_metadata.client_id;

                        // Create SseEvent::Message
                        let sse_event = SseEvent::Message(message);

                        // Send event to the specific client
                        Self::send_to_client(clients, &client_id, sse_event).await?;
                    } else {
                        return Err(SseError::Other("Invalid metadata type provided".to_string()).into());
                    }

                    Ok(())
                }
                SseMode::Client { message_endpoint, http_client, .. } => {
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

                    // Serialize the message
                    let message_str = serde_json::to_string(&message).context("Failed to serialize JsonRpcMessage")?;

                    // Send HTTP POST request (client_id is part of the URL path)
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

                    // Send a shutdown event to all connected clients
                    for (client_id, tx) in clients_map.drain() {
                        debug!("Sending shutdown event to client {}", client_id.to_string());

                        // Create shutdown system message
                        let shutdown_event =
                            SseEvent::Shutdown(Shutdown { reason: "Server is shutting down".to_string() });

                        // Send the shutdown event to the client
                        if tx.send(shutdown_event).await.is_err() {
                            debug!("Client {} already disconnected", client_id.to_string());
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
