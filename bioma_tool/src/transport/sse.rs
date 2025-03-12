use super::Transport;
use crate::client::ServerConfig;
use actix_web::{get, post, web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use actix_web_lab::sse::{Data, Event, Sse};
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info};
use uuid::Uuid;

// ===== Message Types =====

/// Endpoint message sent from server to client on connection
#[derive(Serialize, Deserialize)]
struct EndpointMessage {
    #[serde(rename = "type")]
    message_type: String,
    uri: String,
    #[serde(rename = "clientId")]
    client_id: String,
}

impl EndpointMessage {
    fn new(client_id: &str) -> Self {
        Self { message_type: "endpoint".to_string(), uri: "/message".to_string(), client_id: client_id.to_string() }
    }
}

// ===== Type definitions for improved type safety =====

/// Client identifier type
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ClientId(String);

impl ClientId {
    fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<ClientId> for String {
    fn from(id: ClientId) -> Self {
        id.0
    }
}

/// Server URL type
#[derive(Debug, Clone)]
struct ServerUrl(String);

impl ServerUrl {
    fn from_args(args: &[String]) -> Option<Self> {
        args.iter().find_map(|arg| if arg.starts_with("--url=") { Some(Self(arg[6..].to_string())) } else { None })
    }

    fn events_endpoint(&self) -> String {
        format!("{}/events", self.0)
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

/// Post URL for client to send messages
#[derive(Debug, Clone)]
struct PostUrl(String);

impl PostUrl {
    fn from_server_and_uri(server_url: &str, uri: &str) -> Self {
        let url = if uri.starts_with("http") { uri.to_string() } else { format!("{}{}", server_url, uri) };
        Self(url)
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

// ===== Custom error type =====

#[derive(Debug, thiserror::Error)]
pub enum SseError {
    #[error("Missing server URL in args (expected --url=http://...)")]
    MissingServerUrl,

    #[error("Failed to create HTTP client: {0}")]
    HttpClientCreation(#[from] reqwest::Error),

    #[error("Server initialization error: {0}")]
    ServerInitialization(String),

    #[error("Cannot send message: post URL not yet received from server")]
    PostUrlNotReceived,

    #[error("Failed to connect to SSE endpoint: {0}")]
    ConnectionFailed(String),

    #[error("Server returned error status: {0}")]
    ServerError(String),

    #[error("Failed to parse SSE event: {0}")]
    EventParseError(#[from] serde_json::Error),

    #[error("SSE connection error: {0}")]
    ConnectionError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

// Create a Result type alias for our custom error
type Result<T> = std::result::Result<T, SseError>;

// ===== SSE mode variants =====

/// Represents SSE transport mode (client or server)
enum SseMode {
    /// Server mode manages one or more client connections
    Server {
        /// Server address to bind to
        address: SocketAddr,
        /// Channel to notify when new messages arrive from any client
        request_tx: Option<mpsc::Sender<String>>,
        /// Channels to send messages to connected clients
        clients: Arc<RwLock<HashMap<ClientId, mpsc::Sender<String>>>>,
    },
    /// Client mode connects to an SSE endpoint
    Client {
        /// Server URL for SSE connection
        server_url: ServerUrl,
        /// URL to send messages to (provided by server on connection)
        post_url: Arc<RwLock<Option<PostUrl>>>,
        /// HTTP client for sending messages
        http_client: HttpClient,
    },
}

/// Configuration for SSE timeouts
#[derive(Clone)]
struct SseTimeoutConfig {
    /// Connection timeout for HTTP requests
    connection_timeout: Duration,
    /// SSE event read timeout
    read_timeout: Duration,
}

impl Default for SseTimeoutConfig {
    fn default() -> Self {
        Self { connection_timeout: Duration::from_secs(120), read_timeout: Duration::from_secs(120) }
    }
}

// ===== Main transport implementation =====

#[derive(Clone)]
pub struct SseTransport {
    mode: Arc<RwLock<SseMode>>,
    timeout_config: SseTimeoutConfig,
}

impl SseTransport {
    /// Create a new SSE transport in server mode
    pub fn new_server(address: SocketAddr) -> Self {
        Self {
            mode: Arc::new(RwLock::new(SseMode::Server {
                address,
                request_tx: None,
                clients: Arc::new(RwLock::new(HashMap::new())),
            })),
            timeout_config: SseTimeoutConfig::default(),
        }
    }

    /// Create a new SSE transport in client mode
    pub fn new_client(server: &ServerConfig) -> Result<Self> {
        let server_url = ServerUrl::from_args(&server.args).ok_or(SseError::MissingServerUrl)?;

        let http_client =
            HttpClient::builder().timeout(Duration::from_secs(120)).build().map_err(SseError::HttpClientCreation)?;

        Ok(Self {
            mode: Arc::new(RwLock::new(SseMode::Client {
                server_url,
                post_url: Arc::new(RwLock::new(None)),
                http_client,
            })),
            timeout_config: SseTimeoutConfig::default(),
        })
    }
}

// Transport trait implementation
impl Transport for SseTransport {
    async fn start(&mut self, request_tx: mpsc::Sender<String>) -> std::result::Result<(), anyhow::Error> {
        let mode = self.mode.read().await;
        match &*mode {
            SseMode::Server { .. } => {
                drop(mode); // Release the read lock
                self.start_server(request_tx).await
            }
            SseMode::Client { server_url, post_url, http_client } => {
                let server_url = server_url.clone();
                let post_url = Arc::clone(post_url);
                let http_client = http_client.clone();
                let timeout_config = self.timeout_config.clone();

                self.start_client(server_url, post_url, http_client, request_tx, timeout_config).await
            }
        }
        .map_err(|e| anyhow::anyhow!("{}", e))
    }

    async fn send(&mut self, message: String) -> std::result::Result<(), anyhow::Error> {
        let mode = self.mode.read().await;
        match &*mode {
            SseMode::Server { clients, .. } => self.broadcast_to_clients(clients, message).await,
            SseMode::Client { post_url, http_client, .. } => self.send_to_server(post_url, http_client, message).await,
        }
        .map_err(|e| anyhow::anyhow!("{}", e))
    }
}

// ===== Server implementation methods =====

impl SseTransport {
    /// Start the SSE server
    async fn start_server(&mut self, request_tx: mpsc::Sender<String>) -> Result<()> {
        // Update the server with the request_tx
        let mut mode = self.mode.write().await;
        if let SseMode::Server { address, request_tx: server_request_tx, clients } = &mut *mode {
            *server_request_tx = Some(request_tx.clone());
            let clients_clone = Arc::clone(clients);
            let req_tx_clone = request_tx;

            // Start the HTTP server
            let server_addr = *address;
            info!("Starting SSE server on {}", server_addr);

            // Create the server with actix-web
            let clients_data = web::Data::new(clients_clone);
            let req_tx_data = web::Data::new(req_tx_clone);
            let timeout_config = self.timeout_config.clone();

            // Spawn in a separate task
            tokio::spawn(async move {
                let server = HttpServer::new(move || {
                    App::new()
                        .app_data(clients_data.clone())
                        .app_data(req_tx_data.clone())
                        .service(sse_handler)
                        .service(message_handler)
                })
                .keep_alive(timeout_config.connection_timeout)
                .client_request_timeout(timeout_config.connection_timeout)
                .bind(server_addr)
                .unwrap()
                .run();

                if let Err(e) = server.await {
                    error!("SSE server error: {}", e);
                }
            });

            // Server continues running in the background
            Ok(())
        } else {
            Err(SseError::ServerInitialization("Failed to initialize server".into()))
        }
    }

    /// Broadcast a message to all connected clients
    async fn broadcast_to_clients(
        &self,
        clients: &Arc<RwLock<HashMap<ClientId, mpsc::Sender<String>>>>,
        message: String,
    ) -> Result<()> {
        let client_count = clients.read().await.len();
        debug!("Server broadcasting message to {} clients", client_count);

        // Get all connected clients
        let client_senders = clients.read().await;

        // Send the message to all clients
        for (client_id, tx) in client_senders.iter() {
            if let Err(e) = tx.send(message.clone()).await {
                error!("Failed to send message to client {}: {}", client_id.as_str(), e);
            }
        }

        Ok(())
    }

    /// Handler for SSE connections - this implements the MCP SSE endpoint requirement
    async fn handle_sse_connection(
        client_id: ClientId,
        clients: Arc<RwLock<HashMap<ClientId, mpsc::Sender<String>>>>,
    ) -> impl Responder {
        let (tx, mut rx) = mpsc::channel(100);

        // Store the client in the connected clients map
        {
            let mut clients_map = clients.write().await;
            clients_map.insert(client_id.clone(), tx.clone());
            info!("New SSE client connected: {}", client_id.as_str());
        }

        // Send the initial endpoint event containing the URI
        let endpoint_message = EndpointMessage::new(client_id.as_str());
        let endpoint_json = serde_json::to_string(&endpoint_message)
            .unwrap_or_else(|_| r#"{"type":"endpoint","uri":"/message","clientId":"unknown"}"#.to_string());

        if let Err(e) = tx.send(endpoint_json).await {
            error!("Failed to send endpoint message to client {}: {}", client_id.as_str(), e);
        }

        // Create a stream of SSE events
        let clients_clone = clients;
        let client_id_clone = client_id;
        let keepalive_interval = Duration::from_secs(15);

        let stream = async_stream::stream! {
            let mut last_keepalive = std::time::Instant::now();

            while let Some(msg) = rx.recv().await {
                // Send the message as an SSE event
                yield Ok::<_, std::convert::Infallible>(Event::Data(Data::new(msg)));

                // Check if we need to send a keepalive
                let now = std::time::Instant::now();
                if now.duration_since(last_keepalive) >= keepalive_interval {
                    last_keepalive = now;
                    yield Ok::<_, std::convert::Infallible>(Event::Comment("keepalive".into()));
                }
            }

            // When the loop exits, the client has disconnected
            let mut clients = clients_clone.write().await;
            clients.remove(&client_id_clone);
            info!("SSE client disconnected: {}", client_id_clone.as_str());
        };

        // Return the SSE stream with a longer keepalive duration
        Sse::from_stream(stream).with_keep_alive(Duration::from_secs(25))
    }
}

// ===== Client implementation methods =====

impl SseTransport {
    /// Start the SSE client
    async fn start_client(
        &self,
        server_url: ServerUrl,
        post_url: Arc<RwLock<Option<PostUrl>>>,
        http_client: HttpClient,
        request_tx: mpsc::Sender<String>,
        timeout_config: SseTimeoutConfig,
    ) -> Result<()> {
        // Spawn a task to handle SSE connection with reconnection logic
        tokio::spawn(async move {
            loop {
                debug!("Connecting to SSE endpoint: {}", server_url.events_endpoint());

                match connect_and_process_sse(
                    &server_url,
                    post_url.clone(),
                    &http_client,
                    request_tx.clone(),
                    &timeout_config,
                )
                .await
                {
                    Ok(()) => {
                        debug!("SSE connection closed, reconnecting in 3 seconds...");
                    }
                    Err(e) => {
                        error!("SSE connection error: {}, reconnecting in 3 seconds...", e);
                    }
                }

                // Wait before reconnecting
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        });

        Ok(())
    }

    /// Send a message to the server
    async fn send_to_server(
        &self,
        post_url: &Arc<RwLock<Option<PostUrl>>>,
        http_client: &HttpClient,
        message: String,
    ) -> Result<()> {
        // Get the post URL
        let url = {
            let url_guard = post_url.read().await;
            match &*url_guard {
                Some(url) => url.clone(),
                None => {
                    return Err(SseError::PostUrlNotReceived);
                }
            }
        };

        debug!("Client sending message to {}", url.as_str());

        // Send the message as JSON with increased timeout
        let response = http_client
            .post(url.as_str())
            .header("Content-Type", "application/json")
            .timeout(self.timeout_config.connection_timeout)
            .body(message)
            .send()
            .await
            .map_err(|e| SseError::ConnectionFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(SseError::ServerError(response.status().to_string()));
        }

        Ok(())
    }
}

// ===== Actix-web handlers =====

/// Handler for SSE connections - this implements the MCP SSE endpoint requirement
#[get("/events")]
async fn sse_handler(
    _req: HttpRequest,
    clients: web::Data<Arc<RwLock<HashMap<ClientId, mpsc::Sender<String>>>>>,
) -> impl Responder {
    let client_id = ClientId::new();
    SseTransport::handle_sse_connection(client_id, clients.get_ref().clone()).await
}

/// Handler for receiving messages from clients
#[post("/message")]
async fn message_handler(request_tx: web::Data<mpsc::Sender<String>>, body: web::Bytes) -> impl Responder {
    let body_str = String::from_utf8_lossy(&body);
    debug!("Received message from client: {}", body_str);

    // Forward the message to the request handler
    if let Err(e) = request_tx.send(body_str.to_string()).await {
        error!("Failed to forward client message: {}", e);
        return HttpResponse::InternalServerError().finish();
    }

    HttpResponse::Ok().finish()
}

// ===== SSE Client Connection Handling =====

/// Connect to an SSE endpoint and process incoming events
async fn connect_and_process_sse(
    server_url: &ServerUrl,
    post_url: Arc<RwLock<Option<PostUrl>>>,
    http_client: &HttpClient,
    request_tx: mpsc::Sender<String>,
    timeout_config: &SseTimeoutConfig,
) -> Result<()> {
    debug!("Attempting to connect to SSE URL: {}", server_url.events_endpoint());

    // Establish the SSE connection
    let response = http_client
        .get(&server_url.events_endpoint())
        .timeout(timeout_config.connection_timeout)
        .header("Accept", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .send()
        .await
        .map_err(|e| SseError::ConnectionFailed(e.to_string()))?;

    if !response.status().is_success() {
        return Err(SseError::ServerError(response.status().to_string()));
    }

    debug!("Successfully connected to SSE endpoint");

    // Process the SSE stream
    process_sse_stream(response, post_url, request_tx, timeout_config).await
}

/// Process the SSE stream from the server
async fn process_sse_stream(
    mut response: reqwest::Response,
    post_url: Arc<RwLock<Option<PostUrl>>>,
    request_tx: mpsc::Sender<String>,
    timeout_config: &SseTimeoutConfig,
) -> Result<()> {
    let mut buffer = String::new();
    let start_time = std::time::Instant::now();

    // Read and process chunks from the SSE stream
    while let Some(chunk) = response.chunk().await.map_err(|e| SseError::ConnectionError(e.to_string()))? {
        // Check for timeout
        let current_time = std::time::Instant::now();
        if current_time.duration_since(start_time) > timeout_config.read_timeout {
            error!("Connection timeout reached, reconnecting");
            break;
        }

        // Process the chunk
        let chunk_str = String::from_utf8_lossy(&chunk);
        debug!("Received chunk: {}", chunk_str);
        buffer.push_str(&chunk_str);

        // Process complete events in the buffer
        process_sse_buffer(&mut buffer, &post_url, &request_tx).await?;
    }

    debug!("SSE connection closed");
    Ok(())
}

/// Process the SSE buffer looking for complete events
async fn process_sse_buffer(
    buffer: &mut String,
    post_url: &Arc<RwLock<Option<PostUrl>>>,
    request_tx: &mpsc::Sender<String>,
) -> Result<()> {
    // Process complete events
    while let Some(pos) = buffer.find("\n\n") {
        let event = buffer[..pos].trim().to_string();
        *buffer = buffer[pos + 2..].to_string();

        // Skip empty events and keepalives
        if event.is_empty() || event.starts_with(":") || event.contains("keepalive") {
            if event.contains("keepalive") {
                debug!("Received keepalive");
            }
            continue;
        }

        debug!("Processing SSE event: {}", event);
        process_sse_event(event, post_url, request_tx).await?;
    }

    Ok(())
}

/// Process an SSE event according to MCP specification
async fn process_sse_event(
    event: String,
    post_url: &Arc<RwLock<Option<PostUrl>>>,
    request_tx: &mpsc::Sender<String>,
) -> Result<()> {
    // Parse the event
    let (event_type, data) = parse_sse_event(&event)?;

    if let Some(data) = data {
        match event_type {
            "endpoint" => {
                handle_endpoint_event(data, post_url).await?;
            }
            "message" => {
                handle_message_event(data, request_tx).await?;
            }
            _ => {
                debug!("Ignoring unknown event type: {}", event_type);
            }
        }
    }

    Ok(())
}

// ===== SSE Event Parsing and Handling =====

/// Parse an SSE event into its type and data
fn parse_sse_event(event: &str) -> Result<(&str, Option<&str>)> {
    let mut event_type = "message";
    let mut data = None;

    for line in event.lines() {
        if line.starts_with("event:") {
            event_type = line.trim_start_matches("event:").trim();
        } else if line.starts_with("data:") {
            data = Some(line.trim_start_matches("data:").trim());
        }
    }

    // Check if this is an endpoint message by looking at the "type" field
    if let Some(data_str) = data {
        if let Ok(json_data) = serde_json::from_str::<serde_json::Value>(data_str) {
            if let Some(type_field) = json_data.get("type").and_then(|t| t.as_str()) {
                if type_field == "endpoint" {
                    debug!("Found endpoint event in message: {}", data_str);
                    event_type = "endpoint";
                }
            }
        }
    }

    Ok((event_type, data))
}

/// Handle an endpoint event from the server
async fn handle_endpoint_event(data: &str, post_url: &Arc<RwLock<Option<PostUrl>>>) -> Result<()> {
    let endpoint: serde_json::Value = serde_json::from_str(data)?;

    if let Some(uri) = endpoint.get("uri").and_then(|v| v.as_str()) {
        // Use the server URL or a default if not available
        let server_base = endpoint
            .get("clientId")
            .and_then(|v| v.as_str())
            .map(|_| "http://127.0.0.1:12345")
            .unwrap_or("http://127.0.0.1:12345");

        let post_url_value = PostUrl::from_server_and_uri(server_base, uri);
        debug!("Received endpoint URI: {}", post_url_value.as_str());

        *post_url.write().await = Some(post_url_value);
    }

    Ok(())
}

/// Handle a message event from the server
async fn handle_message_event(data: &str, request_tx: &mpsc::Sender<String>) -> Result<()> {
    debug!("Received SSE message: {}", data);

    if let Err(e) = request_tx.send(data.to_string()).await {
        error!("Failed to forward SSE message: {}", e);
    }

    Ok(())
}
