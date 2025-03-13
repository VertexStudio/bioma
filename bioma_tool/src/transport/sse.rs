use super::Transport;
use anyhow::{Context, Result};
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
use tokio::sync::{mpsc, Mutex};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, info};
use url::Url;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SseServerConfig {
    pub url: SocketAddr,
}

enum SseMode {
    Server { clients: Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>, url: SocketAddr },
    Client { sse_url: Url, endpoint_url: Arc<Mutex<Option<String>>>, http_client: Client },
}

#[derive(Clone)]
pub struct SseTransport {
    mode: Arc<SseMode>,
}

impl SseTransport {
    pub fn new_server(url: SocketAddr) -> Self {
        let clients = Arc::new(Mutex::new(HashMap::new()));

        Self { mode: Arc::new(SseMode::Server { clients, url }) }
    }

    pub fn new_client(server: &SseServerConfig) -> Result<Self> {
        let http_client = ClientBuilder::new().build().context("Failed to create HTTP client")?;
        let sse_url = Url::parse(&format!("http://{}", server.url)).context("Failed to create SSE URL")?;

        Ok(Self { mode: Arc::new(SseMode::Client { sse_url, endpoint_url: Arc::new(Mutex::new(None)), http_client }) })
    }
}

// Helper to format SSE events
fn format_sse_event(event_type: &str, data: &str) -> String {
    format!("event: {}\ndata: {}\n\n", event_type, data)
}

impl Transport for SseTransport {
    async fn start(&mut self, request_tx: mpsc::Sender<String>) -> Result<()> {
        match &*self.mode {
            SseMode::Server { clients, url } => {
                info!("Starting SSE server on {}", url);

                // Start HTTP server
                let listener = tokio::net::TcpListener::bind(*url).await?;

                // Process incoming connections
                loop {
                    let (stream, _) = listener.accept().await?;
                    let io = TokioIo::new(stream);

                    // Clone everything needed for the connection handler
                    let clients_clone = clients.clone();
                    let request_tx_clone = request_tx.clone();
                    let url_clone = *url;

                    // Spawn a task to serve the connection
                    tokio::task::spawn(async move {
                        // Create HTTP service to handle SSE connections and message receiving
                        let service = service_fn(move |req: Request<hyper::body::Incoming>| {
                            let clients = clients_clone.clone();
                            let request_tx = request_tx_clone.clone();
                            let url = url_clone;

                            async move {
                                match (req.method(), req.uri().path()) {
                                    // SSE endpoint for clients to connect and receive events
                                    (&Method::GET, "/") => {
                                        debug!("New SSE client connected");

                                        // Create a channel for sending messages to this client
                                        let (client_tx, mut client_rx) = mpsc::channel::<String>(32);
                                        let client_id = Uuid::new_v4().to_string();

                                        // Register client
                                        {
                                            let mut clients_map = clients.lock().await;
                                            clients_map.insert(client_id.clone(), client_tx.clone());
                                        }

                                        // Create a new channel for the streaming response
                                        let (response_tx, response_rx) =
                                            mpsc::channel::<Result<Frame<Bytes>, std::io::Error>>(32);

                                        // Spawn a task to handle sending SSE events to the client
                                        tokio::spawn(async move {
                                            // Send initial endpoint event
                                            let endpoint_url = format!("http://{}/message", url);
                                            let endpoint_data = serde_json::json!({ "url": endpoint_url });
                                            let endpoint_event = format_sse_event(
                                                "endpoint",
                                                &serde_json::to_string(&endpoint_data).unwrap_or_default(),
                                            );

                                            // Send the initial event to the client via the response channel
                                            if let Err(_) =
                                                response_tx.send(Ok(Frame::data(Bytes::from(endpoint_event)))).await
                                            {
                                                error!("Failed to send initial endpoint event to response stream");
                                                return;
                                            }

                                            // Process incoming events from the client_rx channel
                                            while let Some(event) = client_rx.recv().await {
                                                if let Err(_) =
                                                    response_tx.send(Ok(Frame::data(Bytes::from(event)))).await
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

                                        response.headers_mut().insert(
                                            header::CONTENT_TYPE,
                                            header::HeaderValue::from_static("text/event-stream"),
                                        );

                                        // Add standard SSE headers for best practices
                                        response.headers_mut().insert(
                                            header::CACHE_CONTROL,
                                            header::HeaderValue::from_static("no-cache"),
                                        );
                                        response
                                            .headers_mut()
                                            .insert(header::CONNECTION, header::HeaderValue::from_static("keep-alive"));

                                        Ok::<_, anyhow::Error>(response)
                                    }
                                    // Message endpoint for receiving client messages
                                    (&Method::POST, "/message") => {
                                        // Get message from request body
                                        let body = req.into_body();
                                        let bytes = body.collect().await?.to_bytes();
                                        let message = String::from_utf8_lossy(&bytes).to_string();

                                        debug!("Received client message: {}", message);

                                        // Forward to message handler
                                        if let Err(e) = request_tx.send(message).await {
                                            error!("Failed to forward message: {}", e);
                                        }

                                        // Return OK response
                                        let response = Response::builder()
                                            .status(StatusCode::OK)
                                            .body(http_body_util::Either::Right(Empty::new()))
                                            .unwrap();

                                        Ok::<_, anyhow::Error>(response)
                                    }
                                    // Any other endpoint
                                    _ => {
                                        let response = Response::builder()
                                            .status(StatusCode::NOT_FOUND)
                                            .body(http_body_util::Either::Right(Empty::new()))
                                            .unwrap();

                                        Ok::<_, anyhow::Error>(response)
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
            SseMode::Client { sse_url, endpoint_url, http_client } => {
                info!("Starting SSE client, connecting to {}", sse_url);

                // Connect to SSE endpoint
                let response = http_client
                    .get(sse_url.to_string())
                    .header("Accept", "text/event-stream")
                    .send()
                    .await
                    .context("Failed to connect to SSE endpoint")?;

                if !response.status().is_success() {
                    return Err(anyhow::anyhow!("Failed to connect to SSE endpoint: HTTP {}", response.status()));
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

                        // Parse event fields
                        let mut event_type = None;
                        let mut event_data = None;

                        for line in event.lines() {
                            if let Some(data) = line.strip_prefix("data: ") {
                                event_data = Some(data.to_string());
                            } else if let Some(typ) = line.strip_prefix("event: ") {
                                event_type = Some(typ.to_string());
                            }
                        }

                        match (event_type, event_data) {
                            // Handle endpoint event - get the URL for sending messages
                            (Some(typ), Some(data)) if typ == "endpoint" => {
                                debug!("Received endpoint event: {}", data);

                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                                    if let Some(url) = json.get("url").and_then(|v| v.as_str()) {
                                        let mut endpoint_guard = endpoint_url.lock().await;
                                        *endpoint_guard = Some(url.to_string());

                                        debug!("Endpoint URL set to: {}", url);
                                    }
                                }
                            }
                            // Handle message event - forward to handler
                            (Some(typ), Some(data)) if typ == "message" => {
                                debug!("Received message event: {}", data);

                                if let Err(e) = request_tx.send(data).await {
                                    error!("Failed to forward message: {}", e);
                                    break;
                                }
                            }
                            // Ignore other event types
                            _ => {}
                        }
                    }
                }

                error!("SSE connection closed");

                // In a production implementation, would attempt to reconnect

                Ok(())
            }
        }
    }

    async fn send(&mut self, message: String) -> Result<()> {
        match &*self.mode {
            SseMode::Server { clients, .. } => {
                debug!("Server sending [sse]: {}", message);

                // Format as SSE message event
                let sse_message = format_sse_event("message", &message);

                // Send to all connected clients
                let clients_map = clients.lock().await;
                let mut disconnected = Vec::new();

                for (client_id, tx) in clients_map.iter() {
                    if let Err(_) = tx.send(sse_message.clone()).await {
                        debug!("Client {} disconnected", client_id);
                        disconnected.push(client_id.clone());
                    }
                }

                // In a production implementation, would clean up disconnected clients
                // This would require dropping the lock first and acquiring it again

                Ok(())
            }
            SseMode::Client { endpoint_url, http_client, .. } => {
                debug!("Client sending [sse]: {}", message);

                // Get endpoint URL
                let url = {
                    let endpoint_guard = endpoint_url.lock().await;
                    match &*endpoint_guard {
                        Some(url) => url.clone(),
                        None => {
                            return Err(anyhow::anyhow!(
                                "No endpoint URL available yet. Wait for the SSE connection to establish."
                            ));
                        }
                    }
                };

                // Send HTTP POST request
                let response = http_client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .body(message)
                    .send()
                    .await
                    .context("Failed to send message")?;

                if !response.status().is_success() {
                    return Err(anyhow::anyhow!("Failed to send message: HTTP {}", response.status()));
                }

                debug!("Message sent successfully");

                Ok(())
            }
        }
    }
}
