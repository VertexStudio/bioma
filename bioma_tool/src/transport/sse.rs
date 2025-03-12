use super::Transport;
use crate::client::ServerConfig;
use actix_web::{get, post, web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use actix_web_lab::sse::{Data, Event, Sse};
use anyhow::{anyhow, Context, Result};
use reqwest::Client as HttpClient;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info};
use uuid::Uuid;

/// Represents SSE transport mode (client or server)
enum SseMode {
    /// Server mode manages one or more client connections
    Server {
        /// Server address to bind to
        address: SocketAddr,
        /// Channel to notify when new messages arrive from any client
        request_tx: Option<mpsc::Sender<String>>,
        /// Channels to send messages to connected clients
        clients: Arc<RwLock<HashMap<String, mpsc::Sender<String>>>>,
    },
    /// Client mode connects to an SSE endpoint
    Client {
        /// Server URL for SSE connection
        server_url: String,
        /// URL to send messages to (provided by server on connection)
        post_url: Arc<RwLock<Option<String>>>,
        /// HTTP client for sending messages
        http_client: HttpClient,
    },
}

#[derive(Clone)]
pub struct SseTransport {
    mode: Arc<RwLock<SseMode>>,
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
        }
    }

    /// Create a new SSE transport in client mode
    pub fn new_client(server: &ServerConfig) -> Result<Self> {
        // Extract server URL from args (expected format: "--url=http://127.0.0.1:12345")
        let server_url = server
            .args
            .iter()
            .find_map(|arg| if arg.starts_with("--url=") { Some(arg[6..].to_string()) } else { None })
            .ok_or_else(|| anyhow!("Missing server URL in args (expected --url=http://...)"))?;

        let http_client =
            HttpClient::builder().timeout(Duration::from_secs(120)).build().context("Failed to create HTTP client")?;

        Ok(Self {
            mode: Arc::new(RwLock::new(SseMode::Client {
                server_url,
                post_url: Arc::new(RwLock::new(None)),
                http_client,
            })),
        })
    }
}

impl Transport for SseTransport {
    async fn start(&mut self, request_tx: mpsc::Sender<String>) -> Result<()> {
        let mode = self.mode.read().await;
        match &*mode {
            SseMode::Server { .. } => {
                drop(mode); // Release the read lock

                // Update the server with the request_tx
                let mut mode = self.mode.write().await;
                if let SseMode::Server { address, request_tx: server_request_tx, clients } = &mut *mode {
                    *server_request_tx = Some(request_tx.clone());
                    let clients_clone = Arc::clone(clients);
                    let req_tx_clone = request_tx.clone();

                    // Start the HTTP server - we'll wait on this future directly
                    let server_addr = *address;
                    info!("Starting SSE server on {}", server_addr);

                    // Create the server with actix-web
                    let clients_data = web::Data::new(Arc::clone(&clients_clone));
                    let req_tx_data = web::Data::new(req_tx_clone);

                    // Spawn in a separate task and keep the main task running
                    tokio::spawn(async move {
                        let server = HttpServer::new(move || {
                            App::new()
                                .app_data(clients_data.clone())
                                .app_data(req_tx_data.clone())
                                .service(sse_handler)
                                .service(message_handler)
                        })
                        .keep_alive(Duration::from_secs(120))
                        .client_request_timeout(Duration::from_secs(120))
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
                    Err(anyhow!("Failed to initialize server"))
                }
            }
            SseMode::Client { server_url, post_url, http_client } => {
                let server_url = server_url.clone();
                let post_url_clone = post_url.clone();
                let http_client = http_client.clone();

                // Start an event source to receive SSE events
                debug!("Connecting to SSE endpoint: {}/events", server_url);

                // Spawn a task to handle SSE connection
                tokio::spawn(async move {
                    loop {
                        match connect_to_sse(&server_url, post_url_clone.clone(), &http_client, request_tx.clone())
                            .await
                        {
                            Ok(()) => {
                                // Connection closed successfully
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
        }
    }

    async fn send(&mut self, message: String) -> Result<()> {
        let mode = self.mode.read().await;
        match &*mode {
            SseMode::Server { clients, .. } => {
                debug!("Server broadcasting message to {} clients", clients.read().await.len());

                // Get all connected clients
                let client_senders = clients.read().await;

                // Send the message to all clients
                for (client_id, tx) in client_senders.iter() {
                    if let Err(e) = tx.send(message.clone()).await {
                        error!("Failed to send message to client {}: {}", client_id, e);
                    }
                }

                Ok(())
            }
            SseMode::Client { post_url, http_client, server_url: _ } => {
                // Get the post URL
                let url = {
                    let url_guard = post_url.read().await;
                    match &*url_guard {
                        Some(url) => url.clone(),
                        None => {
                            return Err(anyhow!("Cannot send message: post URL not yet received from server"));
                        }
                    }
                };

                debug!("Client sending message to {}", url);

                // Send the message as JSON with increased timeout
                let response = http_client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .timeout(Duration::from_secs(120))
                    .body(message)
                    .send()
                    .await
                    .context("Failed to send message to server")?;

                if !response.status().is_success() {
                    return Err(anyhow!("Server returned error status: {}", response.status()));
                }

                Ok(())
            }
        }
    }
}

/// Handler for SSE connections - this implements the MCP SSE endpoint requirement
#[get("/events")]
async fn sse_handler(
    _req: HttpRequest,
    clients: web::Data<Arc<RwLock<HashMap<String, mpsc::Sender<String>>>>>,
) -> impl Responder {
    let client_id = Uuid::new_v4().to_string();
    let (tx, mut rx) = mpsc::channel(100);

    // Store the client in the connected clients map
    {
        let mut clients = clients.write().await;
        clients.insert(client_id.clone(), tx.clone());
        info!("New SSE client connected: {}", client_id);
    }

    // Send the initial endpoint event containing the URI for the client to use
    // This follows the MCP spec requirement to send an endpoint event on connection
    let endpoint_message = format!("{{\"type\":\"endpoint\",\"uri\":\"/message\",\"clientId\":\"{}\"}}", client_id);
    if let Err(e) = tx.send(endpoint_message).await {
        error!("Failed to send endpoint message to client {}: {}", client_id, e);
    }

    // Create a stream of SSE events
    let clients_clone = clients.clone();
    let client_id_clone = client_id.clone();

    let stream = async_stream::stream! {
        // Send keepalive events every 15 seconds to maintain connection
        let keepalive_interval = Duration::from_secs(15);
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

        // When the loop exits, the client has disconnected (rx channel closed)
        // Remove the client from the connected clients map
        let mut clients = clients_clone.write().await;
        clients.remove(&client_id_clone);
        info!("SSE client disconnected: {}", client_id_clone);
    };

    // Return the SSE stream with a longer keepalive duration
    Sse::from_stream(stream).with_keep_alive(Duration::from_secs(25))
}

/// Handler for receiving messages from clients - this implements the MCP HTTP POST endpoint requirement
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

/// Connect to an SSE endpoint and process incoming events
async fn connect_to_sse(
    server_url: &str,
    post_url: Arc<RwLock<Option<String>>>,
    http_client: &HttpClient,
    request_tx: mpsc::Sender<String>,
) -> Result<()> {
    let sse_url = format!("{}/events", server_url);

    debug!("Attempting to connect to SSE URL: {}", sse_url);

    let response = match http_client
        .get(&sse_url)
        .timeout(Duration::from_secs(120))
        .header("Accept", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            return Err(anyhow!("Failed to connect to SSE endpoint: {}", e));
        }
    };

    if !response.status().is_success() {
        return Err(anyhow!("Server returned error status: {}", response.status()));
    }

    debug!("Successfully connected to SSE endpoint");

    let mut response = response;
    let mut buffer = String::new();
    let timeout_duration = Duration::from_secs(120);
    let start_time = std::time::Instant::now();

    // Set a longer timeout for reading chunks
    while let Some(chunk) = response.chunk().await.context("Failed to read SSE chunk")? {
        // Update activity time
        let current_time = std::time::Instant::now();

        // Check if we've been inactive for too long
        if current_time.duration_since(start_time) > timeout_duration {
            error!("Connection timeout reached, reconnecting");
            break;
        }

        let chunk_str = String::from_utf8_lossy(&chunk);
        debug!("Received chunk: {}", chunk_str);
        buffer.push_str(&chunk_str);

        // Process complete events
        while let Some(pos) = buffer.find("\n\n") {
            let event = buffer[..pos].trim().to_string();
            buffer = buffer[pos + 2..].to_string();

            // Skip empty events
            if event.is_empty() {
                continue;
            }

            // Skip keepalive comments but update activity time
            if event.starts_with(":") || event.contains("keepalive") {
                debug!("Received keepalive");
                continue;
            }

            debug!("Processing SSE event: {}", event);
            // Process the event
            match process_sse_event(event, post_url.clone(), &request_tx).await {
                Ok(_) => {
                    // Update activity time after successful event processing
                    continue;
                }
                Err(e) => {
                    error!("Error processing SSE event: {}", e);
                }
            }
        }

        // Check if we've been inactive for too long (2 minutes)
        if current_time.duration_since(start_time) > timeout_duration {
            error!("Connection inactive for too long, reconnecting");
            break;
        }
    }

    debug!("SSE connection closed");
    Ok(())
}

/// Process an SSE event according to MCP specification
async fn process_sse_event(
    event: String,
    post_url: Arc<RwLock<Option<String>>>,
    request_tx: &mpsc::Sender<String>,
) -> Result<()> {
    // Parse the event
    let mut event_type = "message";
    let mut data = None;

    for line in event.lines() {
        if line.starts_with("event:") {
            event_type = line.trim_start_matches("event:").trim();
        } else if line.starts_with("data:") {
            data = Some(line.trim_start_matches("data:").trim());
        }
    }

    if let Some(data) = data {
        // Try to parse the data as JSON to check if it's an endpoint message
        if let Ok(json_data) = serde_json::from_str::<serde_json::Value>(data) {
            // Check if this is an endpoint message by looking at the "type" field
            if let Some(type_field) = json_data.get("type").and_then(|t| t.as_str()) {
                if type_field == "endpoint" {
                    debug!("Found endpoint event in message: {}", data);
                    event_type = "endpoint";
                }
            }
        }

        match event_type {
            "endpoint" => {
                // As per MCP spec, when a client connects, the server sends an endpoint event
                // containing a URI for the client to use for sending messages
                let endpoint: serde_json::Value =
                    serde_json::from_str(data).context("Failed to parse endpoint event")?;

                if let Some(uri) = endpoint.get("uri").and_then(|v| v.as_str()) {
                    // Save the post URL
                    let server_url = endpoint
                        .get("clientId")
                        .and_then(|v| v.as_str())
                        .map(|_| "http://127.0.0.1:12345")
                        .unwrap_or("http://127.0.0.1:12345");

                    let full_uri =
                        if uri.starts_with("http") { uri.to_string() } else { format!("{}{}", server_url, uri) };

                    debug!("Received endpoint URI: {}", full_uri);
                    *post_url.write().await = Some(full_uri);
                }
            }
            "message" => {
                // As per MCP spec, server messages are sent as SSE `message` events
                // with the message content encoded as JSON in the event data
                debug!("Received SSE message: {}", data);
                if let Err(e) = request_tx.send(data.to_string()).await {
                    error!("Failed to forward SSE message: {}", e);
                }
            }
            _ => {
                debug!("Ignoring unknown event type: {}", event_type);
            }
        }
    }

    Ok(())
}
