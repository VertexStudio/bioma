use super::{SendMessage, Transport, TransportSender};
use crate::{ConnectionId, JsonRpcMessage};
use anyhow::Result;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use http_body_util::{BodyExt, Empty, Full, StreamBody};
use hyper::{
    body::Incoming,
    header::{HeaderValue, ACCEPT, CONTENT_TYPE},
    service::Service,
    Method, Request, Response, StatusCode,
};
use hyper_tls::HttpsConnector;
use hyper_util::{
    client::legacy::{connect::HttpConnector, Client},
    rt::{TokioExecutor, TokioIo},
    server,
};
use std::{net::SocketAddr, pin::Pin, sync::Arc};
use tokio::{
    net::TcpListener,
    sync::{mpsc, Mutex},
    task::JoinHandle,
};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, info};
use uuid::Uuid;

const MCP_SESSION_ID: &str = "Mcp-Session-Id";
const LAST_EVENT_ID: &str = "Last-Event-ID";
const TEXT_EVENT_STREAM: &str = "text/event-stream";
const APPLICATION_JSON: &str = "application/json";

struct SseConnection {
    tx: mpsc::Sender<String>,
    last_event_id: Option<String>,
    session_id: Option<String>,
}

struct StreamableServerState {
    on_message: mpsc::Sender<super::Message>,
    connections: Arc<Mutex<std::collections::HashMap<ConnectionId, SseConnection>>>,
    session_ids: Arc<Mutex<std::collections::HashMap<String, ConnectionId>>>,
    events: Arc<Mutex<std::collections::VecDeque<(String, JsonRpcMessage)>>>,
}

struct StreamableServer {
    addr: SocketAddr,
    state: Arc<StreamableServerState>,
}

struct StreamableClient {
    server_url: String,
    on_message: mpsc::Sender<JsonRpcMessage>,
    client: Client<HttpsConnector<HttpConnector>, Full<Bytes>>,
    session_id: Arc<Mutex<Option<String>>>,
    connection_id: ConnectionId,
    last_event_id: Arc<Mutex<Option<String>>>,
}

enum StreamableTransportInner {
    Server(StreamableServer),
    Client(StreamableClient),
}

#[derive(Clone)]
pub struct StreamableTransport {
    inner: Arc<Mutex<StreamableTransportInner>>,
    #[allow(unused)]
    on_error: mpsc::Sender<anyhow::Error>,
    #[allow(unused)]
    on_close: mpsc::Sender<()>,
}

#[derive(Clone)]
pub struct StreamableTransportSender {
    inner: Arc<Mutex<StreamableTransportInner>>,
}

impl SendMessage for StreamableTransportSender {
    async fn send(&self, message: JsonRpcMessage, client_id: ConnectionId) -> Result<()> {
        let inner = self.inner.lock().await;
        match &*inner {
            StreamableTransportInner::Server(server) => {
                let state = &server.state;
                let mut connections = state.connections.lock().await;

                if let Some(connection) = connections.get_mut(&client_id) {
                    if let Some(session_id) = &connection.session_id {
                        let session_ids = state.session_ids.lock().await;
                        if !session_ids.contains_key(session_id) {
                            debug!("Session ID no longer valid for connection");
                            return Ok(());
                        }
                    }

                    let json = serde_json::to_string(&message)?;

                    let event_id = Uuid::new_v4().to_string();

                    connection.last_event_id = Some(event_id.clone());

                    let mut events = state.events.lock().await;
                    events.push_back((event_id.clone(), message.clone()));

                    if events.len() > 100 {
                        events.pop_front();
                    }

                    let sse_message = format!("id: {}\ndata: {}\n\n", event_id, json);

                    if let Err(e) = connection.tx.send(sse_message).await {
                        error!("Failed to send SSE message: {}", e);
                    }
                } else {
                    debug!("Client not connected, message cannot be sent");
                }

                Ok(())
            }
            StreamableTransportInner::Client(client) => {
                if client_id != ConnectionId::new(None) && client_id != client.connection_id {
                    debug!("Message not intended for this client, ignoring");
                    return Ok(());
                }

                let json = serde_json::to_string(&message)?;
                let session_id = client.session_id.lock().await.clone();

                let mut req_builder = Request::builder()
                    .method(Method::POST)
                    .uri(&client.server_url)
                    .header(ACCEPT, format!("{}, {}", APPLICATION_JSON, TEXT_EVENT_STREAM))
                    .header(CONTENT_TYPE, APPLICATION_JSON);

                if let Some(sid) = session_id {
                    req_builder = req_builder.header(MCP_SESSION_ID, HeaderValue::from_str(&sid)?);
                }

                debug!("Client sending [streamable-http]: {}", json);

                let req = req_builder.body(Full::new(Bytes::from(json)))?;

                let resp = client.client.request(req).await?;

                if resp.status().is_success() {
                    if let Some(session_header) = resp.headers().get(MCP_SESSION_ID) {
                        let new_session_id = session_header.to_str()?.to_string();
                        let mut session_guard = client.session_id.lock().await;
                        *session_guard = Some(new_session_id.clone());
                        debug!("Received session ID: {}", new_session_id);
                    }

                    if let Some(content_type) = resp.headers().get(CONTENT_TYPE) {
                        if content_type.to_str()?.contains(TEXT_EVENT_STREAM) {
                            debug!("Received SSE stream response");
                            let body = resp.into_body();
                            let on_message = client.on_message.clone();

                            tokio::spawn(async move {
                                if let Err(e) = process_sse_stream(body, on_message).await {
                                    error!("Error processing SSE stream: {}", e);
                                }
                            });
                        } else {
                            debug!("Received JSON response");
                            let body_bytes = resp.into_body().collect().await?.to_bytes();
                            if !body_bytes.is_empty() {
                                let response_msg: JsonRpcMessage = serde_json::from_slice(&body_bytes)?;
                                if let Err(e) = client.on_message.send(response_msg).await {
                                    error!("Failed to send message to handler: {}", e);
                                }
                            }
                        }
                    }
                } else if resp.status() == StatusCode::NOT_FOUND {
                    let mut session_guard = client.session_id.lock().await;
                    *session_guard = None;
                    error!("Session expired, need to reinitialize");
                }

                Ok(())
            }
        }
    }
}

struct ServerService {
    state: Arc<StreamableServerState>,
}

impl Service<Request<Incoming>> for ServerService {
    type Response = Response<BoxBody>;
    type Error = anyhow::Error;
    type Future = Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: Request<Incoming>) -> Self::Future {
        let state = self.state.clone();

        Box::pin(async move {
            let method = req.method().clone();
            let headers = req.headers().clone();

            let session_id = headers.get(MCP_SESSION_ID).and_then(|v| v.to_str().ok()).map(String::from);

            let conn_id = if let Some(sid) = &session_id {
                let session_ids = state.session_ids.lock().await;
                session_ids.get(sid).cloned().unwrap_or_else(|| ConnectionId::new(None))
            } else {
                ConnectionId::new(None)
            };

            match method {
                Method::POST => handle_post_request(req, state, conn_id, session_id).await,
                Method::GET => handle_get_request(req, state, conn_id, session_id).await,
                Method::DELETE => handle_delete_request(state, session_id).await,
                _ => Ok(Response::builder()
                    .status(StatusCode::METHOD_NOT_ALLOWED)
                    .body(box_body(Full::new(Bytes::from("Method not allowed"))))?),
            }
        })
    }
}

impl StreamableTransport {
    pub fn new_server(
        addr: SocketAddr,
        on_message: mpsc::Sender<super::Message>,
        on_error: mpsc::Sender<anyhow::Error>,
        on_close: mpsc::Sender<()>,
    ) -> Self {
        let state = Arc::new(StreamableServerState {
            on_message,
            connections: Arc::new(Mutex::new(std::collections::HashMap::new())),
            session_ids: Arc::new(Mutex::new(std::collections::HashMap::new())),
            events: Arc::new(Mutex::new(std::collections::VecDeque::new())),
        });

        let server = StreamableServer { addr, state };

        Self { inner: Arc::new(Mutex::new(StreamableTransportInner::Server(server))), on_error, on_close }
    }

    pub fn new_client(
        server_url: String,
        on_message: mpsc::Sender<JsonRpcMessage>,
        on_error: mpsc::Sender<anyhow::Error>,
        on_close: mpsc::Sender<()>,
    ) -> Self {
        let https = HttpsConnector::new();
        let client = Client::builder(TokioExecutor::new()).build(https);

        let client_transport = StreamableClient {
            server_url,
            on_message,
            client,
            session_id: Arc::new(Mutex::new(None)),
            connection_id: ConnectionId::new(Some(Uuid::new_v4().to_string())),
            last_event_id: Arc::new(Mutex::new(None)),
        };

        Self { inner: Arc::new(Mutex::new(StreamableTransportInner::Client(client_transport))), on_error, on_close }
    }
}

impl Transport for StreamableTransport {
    async fn start(&mut self) -> Result<JoinHandle<Result<()>>> {
        let inner_clone = self.inner.clone();
        let handle = tokio::spawn(async move {
            let inner = inner_clone.lock().await;
            match &*inner {
                StreamableTransportInner::Server(server) => {
                    let state = server.state.clone();

                    let listener = TcpListener::bind(&server.addr).await?;
                    info!("StreamableTransport server listening on {}", server.addr);

                    loop {
                        let (stream, _) = listener.accept().await?;
                        let io = TokioIo::new(stream);

                        let service = ServerService { state: state.clone() };

                        tokio::spawn(async move {
                            if let Err(err) = server::conn::auto::Builder::new(TokioExecutor::new())
                                .serve_connection(io, service)
                                .await
                            {
                                eprintln!("Error serving connection: {:?}", err);
                            }
                        });
                    }
                }
                StreamableTransportInner::Client(client) => {
                    let session_id = client.session_id.lock().await.clone();
                    let last_event_id = client.last_event_id.lock().await.clone();
                    let on_message = client.on_message.clone();
                    let server_url = client.server_url.clone();
                    let client_instance = client.client.clone();

                    if let Some(sid) = session_id {
                        debug!("Opening SSE stream with session ID: {}", sid);

                        let mut req_builder = Request::builder()
                            .method(Method::GET)
                            .uri(&server_url)
                            .header(ACCEPT, TEXT_EVENT_STREAM)
                            .header(MCP_SESSION_ID, sid);

                        if let Some(event_id) = last_event_id {
                            debug!("Requesting resumption from event ID: {}", event_id);
                            req_builder = req_builder.header(LAST_EVENT_ID, event_id);
                        }

                        let req = req_builder.body(Full::new(Bytes::new()))?;

                        match client_instance.request(req).await {
                            Ok(resp) => {
                                if resp.status().is_success() {
                                    info!("Established SSE connection to server");
                                    let body = resp.into_body();

                                    let last_event_id_mutex = client.last_event_id.clone();
                                    process_sse_stream_with_tracking(body, on_message, last_event_id_mutex).await?;
                                } else if resp.status() == StatusCode::METHOD_NOT_ALLOWED {
                                    debug!("Server does not support GET method for SSE");
                                } else {
                                    debug!("Unexpected status code from SSE stream: {}", resp.status());
                                }
                            }
                            Err(e) => {
                                error!("Failed to establish SSE connection: {}", e);
                            }
                        }
                    } else {
                        debug!("No session ID yet, waiting for initialization");
                    }

                    Ok(())
                }
            }
        });

        Ok(handle)
    }

    async fn send(&mut self, message: JsonRpcMessage, client_id: ConnectionId) -> Result<()> {
        let sender = self.sender();
        sender.send(message, client_id).await
    }

    async fn close(&mut self) -> Result<()> {
        let inner = self.inner.lock().await;
        match &*inner {
            StreamableTransportInner::Server(_) => {
                debug!("Closing StreamableTransport server");
            }
            StreamableTransportInner::Client(client) => {
                if let Some(session_id) = client.session_id.lock().await.as_ref() {
                    debug!("Terminating session: {}", session_id);

                    let req = Request::builder()
                        .method(Method::DELETE)
                        .uri(&client.server_url)
                        .header(MCP_SESSION_ID, session_id.clone())
                        .body(Full::new(Bytes::new()))?;

                    match client.client.request(req).await {
                        Ok(_) => debug!("Session terminated successfully"),
                        Err(e) => debug!("Failed to terminate session: {}", e),
                    }
                }
            }
        }

        Ok(())
    }

    fn sender(&self) -> TransportSender {
        TransportSender::new_streamable(StreamableTransportSender { inner: self.inner.clone() })
    }
}

type BoxBody = http_body_util::combinators::BoxBody<Bytes, anyhow::Error>;

fn box_body<B>(body: B) -> BoxBody
where
    B: http_body_util::BodyExt<Data = Bytes> + Send + Sync + 'static,
    B::Error: std::fmt::Debug,
{
    body.map_err(|err| anyhow::anyhow!("{:?}", err)).boxed()
}

async fn process_sse_stream(body: Incoming, on_message: mpsc::Sender<JsonRpcMessage>) -> Result<()> {
    let mut buffer = String::new();
    let mut data = String::new();
    let mut event_id = String::new();
    let mut last_processed_id = None;

    let mut stream = body;

    while let Some(frame_result) = stream.frame().await {
        let frame = frame_result?;
        if let Some(data_bytes) = frame.data_ref() {
            let chunk_str = String::from_utf8(data_bytes.to_vec())?;

            buffer.push_str(&chunk_str);

            while let Some(pos) = buffer.find("\n\n") {
                let event = buffer[..pos + 2].to_string();
                buffer = buffer[pos + 2..].to_string();

                for line in event.lines() {
                    if line.starts_with("data: ") {
                        data.push_str(&line[6..]);
                    } else if line.starts_with("id: ") {
                        event_id = line[4..].to_string();

                        if event_id != "welcome" {
                            last_processed_id = Some(event_id.clone());
                        }
                    } else if line.is_empty() && !data.is_empty() {
                        debug!("Received SSE event with id: {}", event_id);
                        match serde_json::from_str::<JsonRpcMessage>(&data) {
                            Ok(message) => {
                                if let Err(e) = on_message.send(message).await {
                                    error!("Failed to forward SSE message: {}", e);
                                }
                            }
                            Err(e) => {
                                error!("Failed to parse SSE data as JsonRpcMessage: {}", e);
                            }
                        }

                        data.clear();
                    }
                }
            }
        }
    }

    if let Some(id) = last_processed_id {
        debug!("Last processed event ID: {}", id);
    }

    debug!("SSE stream closed");
    Ok(())
}

async fn process_sse_stream_with_tracking(
    body: Incoming,
    on_message: mpsc::Sender<JsonRpcMessage>,
    last_event_id: Arc<Mutex<Option<String>>>,
) -> Result<()> {
    let mut buffer = String::new();
    let mut data = String::new();
    let mut event_id = String::new();

    let mut stream = body;

    while let Some(frame_result) = stream.frame().await {
        let frame = frame_result?;
        if let Some(data_bytes) = frame.data_ref() {
            let chunk_str = String::from_utf8(data_bytes.to_vec())?;

            buffer.push_str(&chunk_str);

            while let Some(pos) = buffer.find("\n\n") {
                let event = buffer[..pos + 2].to_string();
                buffer = buffer[pos + 2..].to_string();

                for line in event.lines() {
                    if line.starts_with("data: ") {
                        data.push_str(&line[6..]);
                    } else if line.starts_with("id: ") {
                        event_id = line[4..].to_string();

                        if event_id != "welcome" {
                            let mut last_id = last_event_id.lock().await;
                            *last_id = Some(event_id.clone());
                            debug!("Updated last event ID to: {}", event_id);
                        }
                    } else if line.is_empty() && !data.is_empty() {
                        debug!("Received SSE event with id: {}", event_id);
                        match serde_json::from_str::<JsonRpcMessage>(&data) {
                            Ok(message) => {
                                if let Err(e) = on_message.send(message).await {
                                    error!("Failed to forward SSE message: {}", e);
                                }
                            }
                            Err(e) => {
                                error!("Failed to parse SSE data as JsonRpcMessage: {}", e);
                            }
                        }

                        data.clear();
                    }
                }
            }
        }
    }

    debug!("SSE stream closed");
    Ok(())
}

async fn handle_post_request(
    req: Request<Incoming>,
    state: Arc<StreamableServerState>,
    conn_id: ConnectionId,
    session_id: Option<String>,
) -> Result<Response<BoxBody>, anyhow::Error> {
    if let Some(sid) = &session_id {
        let connections = state.connections.lock().await;
        if !connections.is_empty() {
            let matches = verify_session_ownership(&connections, &conn_id, sid);
            if !matches {
                let session_ids = state.session_ids.lock().await;
                if session_ids.contains_key(sid) {
                    debug!("Session ID provided doesn't match the connection ID");
                    return Ok(Response::builder()
                        .status(StatusCode::FORBIDDEN)
                        .body(box_body(Full::new(Bytes::from("Session ID mismatch"))))?);
                }
            }
        }
    }

    let body_bytes = req.into_body().collect().await?.to_bytes();

    let message: Result<JsonRpcMessage, _> = serde_json::from_slice(&body_bytes);

    match message {
        Ok(json_rpc_message) => {
            debug!("Server received [streamable-http]: {}", String::from_utf8_lossy(&body_bytes));

            let is_response_or_notification = match &json_rpc_message {
                JsonRpcMessage::Response(_) => true,
                JsonRpcMessage::Request(req) => match req {
                    jsonrpc_core::Request::Single(call) => match call {
                        jsonrpc_core::Call::Notification(_) => true,
                        _ => false,
                    },
                    jsonrpc_core::Request::Batch(calls) => calls.iter().all(|call| match call {
                        jsonrpc_core::Call::Notification(_) => true,
                        _ => false,
                    }),
                },
            };

            let message = super::Message { message: json_rpc_message.clone(), conn_id: conn_id.clone() };

            if let Err(e) = state.on_message.send(message).await {
                error!("Failed to forward message to handler: {}", e);
                return Ok(Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(box_body(Full::new(Bytes::from("Failed to process message"))))?);
            }

            let mut session_header = None;

            if is_initialize_request(&json_rpc_message) {
                let new_session_id = Uuid::new_v4().to_string();
                debug!("Creating new session ID for initialize request: {}", new_session_id);

                let mut session_ids = state.session_ids.lock().await;
                session_ids.insert(new_session_id.clone(), conn_id.clone());

                session_header = Some(new_session_id);
            } else if let Some(sid) = &session_id {
                let mut session_ids = state.session_ids.lock().await;
                if !session_ids.contains_key(sid) {
                    debug!("Registering existing session ID: {}", sid);
                    session_ids.insert(sid.clone(), conn_id.clone());
                }
            }

            if is_response_or_notification {
                let mut response = Response::builder().status(StatusCode::ACCEPTED);

                if let Some(sid) = session_header {
                    response = response.header(MCP_SESSION_ID, sid);
                }

                return Ok(response.body(box_body(Empty::new()))?);
            } else {
                let (tx, rx) = mpsc::channel::<String>(100);

                let mut connections = state.connections.lock().await;
                let effective_session_id = session_header.clone().or(session_id.clone());

                connections.insert(
                    conn_id.clone(),
                    SseConnection { tx, last_event_id: None, session_id: effective_session_id.clone() },
                );

                if let Some(sid) = session_header.clone() {
                    let mut session_ids = state.session_ids.lock().await;
                    session_ids.insert(sid, conn_id.clone());
                }

                let stream = ReceiverStream::new(rx);

                let body = create_sse_stream(stream);

                let mut response = Response::builder().status(StatusCode::OK).header(CONTENT_TYPE, TEXT_EVENT_STREAM);

                if let Some(sid) = session_header {
                    response = response.header(MCP_SESSION_ID, sid);
                }

                debug!("Starting SSE stream for client request");
                return Ok(response.body(body)?);
            }
        }
        Err(e) => {
            error!("Failed to parse message: {}", e);
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(box_body(Full::new(Bytes::from(format!("Invalid JSON-RPC message: {}", e)))))?);
        }
    }
}

fn create_sse_stream<S>(stream: S) -> BoxBody
where
    S: Stream<Item = String> + Send + Sync + 'static,
{
    let mapped_stream = stream.map(|s| {
        let bytes = Bytes::from(s);
        Ok::<_, anyhow::Error>(hyper::body::Frame::data(bytes))
    });

    let stream_body = StreamBody::new(mapped_stream);

    box_body(stream_body)
}

async fn handle_get_request(
    req: Request<Incoming>,
    state: Arc<StreamableServerState>,
    conn_id: ConnectionId,
    session_id: Option<String>,
) -> Result<Response<BoxBody>, anyhow::Error> {
    let headers = req.headers();
    let accept_header = headers.get(ACCEPT).and_then(|v| v.to_str().ok()).unwrap_or("");

    if !accept_header.contains(TEXT_EVENT_STREAM) {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(box_body(Full::new(Bytes::from("Must accept text/event-stream"))))?);
    }

    debug!("Handling GET request for SSE stream");

    let last_event_id = headers.get(LAST_EVENT_ID).and_then(|v| v.to_str().ok()).map(String::from);

    let (tx, rx) = mpsc::channel::<String>(100);

    let mut connections = state.connections.lock().await;
    connections.insert(
        conn_id.clone(),
        SseConnection { tx: tx.clone(), last_event_id: last_event_id.clone(), session_id: session_id.clone() },
    );

    if let Some(sid) = session_id.clone() {
        let mut session_ids = state.session_ids.lock().await;
        session_ids.insert(sid, conn_id.clone());
    }

    if let Some(last_id) = last_event_id {
        debug!("Client requested replay from event ID: {}", last_id);

        tx.send(format!(
            "id: welcome\ndata: {{\n data: \"Reconnected to server, replaying events from {}\"\n}}\n\n",
            last_id
        ))
        .await?;

        let events = state.events.lock().await;
        let mut found_last_id = false;

        for (id, message) in events.iter() {
            if found_last_id {
                let json = serde_json::to_string(message)?;
                let sse_message = format!("id: {}\ndata: {}\n\n", id, json);

                if let Err(e) = tx.send(sse_message).await {
                    error!("Failed to send replayed SSE message: {}", e);
                    break;
                }
            } else if id == &last_id {
                found_last_id = true;
            }
        }
    } else {
        tx.send("id: welcome\ndata: {\"message\": \"Connected to MCP server\"}\n\n".to_string()).await?;
    }

    let stream = ReceiverStream::new(rx);

    let body = create_sse_stream(stream);

    Ok(Response::builder().status(StatusCode::OK).header(CONTENT_TYPE, TEXT_EVENT_STREAM).body(body)?)
}

async fn handle_delete_request(
    state: Arc<StreamableServerState>,
    session_id: Option<String>,
) -> Result<Response<BoxBody>, anyhow::Error> {
    if let Some(sid) = session_id {
        debug!("Terminating session: {}", sid);
        let mut session_ids = state.session_ids.lock().await;
        if let Some(conn_id) = session_ids.remove(&sid) {
            let mut connections = state.connections.lock().await;

            if let Some(conn) = connections.get(&conn_id) {
                if conn.session_id.as_ref() != Some(&sid) {
                    debug!("Connection session ID mismatch for termination");
                    return Ok(Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(box_body(Full::new(Bytes::from("Session ID mismatch"))))?);
                }
            }

            connections.remove(&conn_id);
        }

        return Ok(Response::builder()
            .status(StatusCode::OK)
            .body(box_body(Full::new(Bytes::from("Session terminated"))))?);
    } else {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(box_body(Full::new(Bytes::from("No session ID provided"))))?);
    }
}

fn verify_session_ownership(
    connections: &std::collections::HashMap<ConnectionId, SseConnection>,
    conn_id: &ConnectionId,
    session_id: &str,
) -> bool {
    if let Some(conn) = connections.get(conn_id) {
        if let Some(conn_sid) = &conn.session_id {
            return conn_sid == session_id;
        }
    }
    false
}

fn is_initialize_request(message: &JsonRpcMessage) -> bool {
    match message {
        JsonRpcMessage::Request(req) => match req {
            jsonrpc_core::Request::Single(call) => match call {
                jsonrpc_core::Call::MethodCall(method_call) => method_call.method == "initialize",
                _ => false,
            },
            _ => false,
        },
        _ => false,
    }
}
