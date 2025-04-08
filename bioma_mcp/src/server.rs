use crate::logging::McpLoggingLayer;
use crate::operation::Operation;
use crate::prompts::PromptGetHandler;
use crate::resources::ResourceReadHandler;
use crate::schema::{
    CallToolRequestParams, CancelledNotificationParams, ClientCapabilities, CreateMessageRequestParams,
    CreateMessageResult, GetPromptRequestParams, Implementation, InitializeRequestParams, InitializeResult,
    InitializedNotificationParams, ListPromptsRequestParams, ListPromptsResult, ListResourceTemplatesRequestParams,
    ListResourceTemplatesResult, ListResourcesRequestParams, ListResourcesResult, ListToolsRequestParams,
    ListToolsResult, PingRequestParams, ReadResourceRequestParams, ResourceUpdatedNotificationParams,
    ServerCapabilities, SubscribeRequestParams, UnsubscribeRequestParams,
};
use crate::tools::ToolCallHandler;
use crate::transport::sse::SseTransport;
use crate::transport::ws::WsTransport;
use crate::transport::{stdio::StdioTransport, Message, Transport, TransportSender, TransportType};
use crate::{ConnectionId, JsonRpcMessage, MessageId, RequestId};

use base64;
use jsonrpc_core::{MetaIoHandler, Metadata, Params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tokio::task::AbortHandle;
use tracing::{debug, error, info, warn};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{Layer, Registry};

#[derive(Clone)]
pub struct ServerMetadata {
    pub conn_id: ConnectionId,
}

impl Metadata for ServerMetadata {}

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("Transport error: {0}")]
    Transport(String),
    #[error("Request error: {0}")]
    Request(String),
    #[error("Failed to parse response: {0}")]
    ParseResponse(String),
}

pub type TracingLayer = Box<dyn Layer<Registry> + Send + Sync + 'static>;

pub trait ModelContextProtocolServer: Send + Sync + 'static {
    fn get_transport_config(&self) -> impl Future<Output = TransportConfig> + Send;
    fn get_capabilities(&self) -> impl Future<Output = ServerCapabilities> + Send;
    fn get_pagination(&self) -> impl Future<Output = Option<Pagination>> + Send;
    fn get_tracing_layer(&self) -> impl Future<Output = Option<TracingLayer>> + Send {
        async { None }
    }
    fn new_resources(&self, context: Context) -> impl Future<Output = Vec<Arc<dyn ResourceReadHandler>>> + Send;
    fn new_prompts(&self, context: Context) -> impl Future<Output = Vec<Arc<dyn PromptGetHandler>>> + Send;
    fn new_tools(&self, context: Context) -> impl Future<Output = Vec<Arc<dyn ToolCallHandler>>> + Send;
    fn on_error(&self, error: anyhow::Error) -> impl Future<Output = ()> + Send;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StdioConfig {}

#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct SseConfig {
    #[builder(default = default_server_url())]
    pub endpoint: String,
    #[builder(default = default_channel_capacity())]
    pub channel_capacity: usize,
}

fn default_server_url() -> String {
    "127.0.0.1:8090".to_string()
}

fn default_channel_capacity() -> usize {
    32
}

impl Default for SseConfig {
    fn default() -> Self {
        Self::builder().build()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct WsConfig {
    #[builder(default = default_ws_server_url())]
    pub endpoint: String,
}

fn default_ws_server_url() -> String {
    "127.0.0.1:9090".to_string()
}

impl Default for WsConfig {
    fn default() -> Self {
        Self::builder().build()
    }
}

#[derive(Debug, Clone)]
pub enum TransportConfig {
    Stdio(StdioConfig),
    Sse(SseConfig),
    Ws(WsConfig),
}

#[derive(Debug, Clone)]
pub struct Pagination {
    pub size: usize,
}

impl Default for Pagination {
    fn default() -> Self {
        Self { size: 20 }
    }
}

impl Pagination {
    pub fn new(size: usize) -> Self {
        Self { size }
    }

    pub fn paginate<T, U, F>(&self, items: &[T], cursor: Option<String>, converter: F) -> (Vec<U>, Option<String>)
    where
        F: Fn(&T) -> U,
    {
        let total_items = items.len();

        if total_items == 0 {
            return (vec![], None);
        }

        let offset = match cursor {
            Some(cursor_str) => self.decode_cursor(&cursor_str).unwrap_or(0),
            None => 0,
        };

        if offset >= total_items {
            return (vec![], None);
        }

        let end = std::cmp::min(offset + self.size, total_items);
        let has_more = end < total_items;

        let result: Vec<U> = items[offset..end].iter().map(&converter).collect();

        let next_cursor = if has_more { Some(self.encode_cursor(end)) } else { None };

        (result, next_cursor)
    }

    pub fn encode_cursor(&self, offset: usize) -> String {
        base64::encode(offset.to_string())
    }

    pub fn decode_cursor(&self, cursor: &str) -> Result<usize, ServerError> {
        let decoded =
            base64::decode(cursor).map_err(|e| ServerError::Request(format!("Invalid cursor format: {}", e)))?;

        let cursor_str =
            String::from_utf8(decoded).map_err(|e| ServerError::Request(format!("Invalid cursor encoding: {}", e)))?;

        let offset =
            cursor_str.parse::<usize>().map_err(|e| ServerError::Request(format!("Invalid cursor value: {}", e)))?;

        Ok(offset)
    }
}

struct Session {
    #[allow(unused)]
    context: Context,
    tools: Vec<Arc<dyn ToolCallHandler>>,
    prompts: Vec<Arc<dyn PromptGetHandler>>,
    resources: Vec<Arc<dyn ResourceReadHandler>>,
}

impl Session {
    fn find_tool(&self, name: &str) -> Option<Arc<dyn ToolCallHandler>> {
        self.tools.iter().find(|tool| tool.def().name == name).cloned()
    }

    fn list_tools(&self, cursor: Option<String>) -> (Vec<crate::schema::Tool>, Option<String>) {
        if let Some(pagination) = &self.context.pagination {
            pagination.paginate(&self.tools, cursor, |tool| tool.def())
        } else {
            (self.tools.iter().map(|tool| tool.def()).collect(), None)
        }
    }

    fn find_resource(&self, uri: &str) -> Option<Arc<dyn ResourceReadHandler>> {
        self.resources.iter().find(|resource| resource.def().uri == uri).cloned()
    }

    fn list_resources(&self, cursor: Option<String>) -> (Vec<crate::schema::Resource>, Option<String>) {
        if let Some(pagination) = &self.context.pagination {
            pagination.paginate(&self.resources, cursor, |resource| resource.def())
        } else {
            (self.resources.iter().map(|resource| resource.def()).collect(), None)
        }
    }

    fn find_prompt(&self, name: &str) -> Option<Arc<dyn PromptGetHandler>> {
        self.prompts.iter().find(|prompt| prompt.def().name == name).cloned()
    }

    fn list_prompts(&self, cursor: Option<String>) -> (Vec<crate::schema::Prompt>, Option<String>) {
        if let Some(pagination) = &self.context.pagination {
            pagination.paginate(&self.prompts, cursor, |prompt| prompt.def())
        } else {
            (self.prompts.iter().map(|prompt| prompt.def()).collect(), None)
        }
    }
}

type ResponseSender = oneshot::Sender<Result<serde_json::Value, ServerError>>;
type RequestCounter = Arc<RwLock<u64>>;
type PendingServerRequests = Arc<Mutex<HashMap<RequestId, ResponseSender>>>;
type PendingClientRequests = Arc<Mutex<HashMap<RequestId, AbortHandle>>>;

#[derive(Clone)]
pub struct Context {
    pub client_capabilities: ClientCapabilities,
    pub server_capabilities: ServerCapabilities,
    conn_id: ConnectionId,
    sender: TransportSender,
    pending_requests: PendingServerRequests,
    request_counter: RequestCounter,
    pagination: Option<Pagination>,
}

impl Context {
    pub fn test() -> Self {
        Self {
            client_capabilities: ClientCapabilities::default(),
            server_capabilities: ServerCapabilities::default(),
            conn_id: ConnectionId::new(None),
            sender: TransportSender::new_nop(),
            pending_requests: PendingServerRequests::default(),
            request_counter: RequestCounter::default(),
            pagination: None,
        }
    }

    pub async fn create_message(
        &self,
        params: CreateMessageRequestParams,
    ) -> Result<Operation<CreateMessageResult>, ServerError> {
        let params = serde_json::to_value(params).unwrap_or_default();
        self.request("sampling/createMessage".to_string(), params, std::time::Duration::from_secs(10)).await
    }

    pub async fn resource_updated(&self, params: ResourceUpdatedNotificationParams) -> Result<(), ServerError> {
        let params = serde_json::to_value(params).unwrap_or_default();
        self.notify("notifications/resources/updated".to_string(), params).await?;
        Ok(())
    }

    pub async fn request<T>(
        &self,
        method: String,
        params: serde_json::Value,
        timeout: std::time::Duration,
    ) -> Result<Operation<T>, ServerError>
    where
        T: DeserializeOwned + Send + 'static,
    {
        let mut counter = self.request_counter.write().await;
        *counter += 1;
        let id = *counter;

        let request = jsonrpc_core::MethodCall {
            jsonrpc: Some(jsonrpc_core::Version::V2),
            method,
            params: Params::Map(params.as_object().cloned().unwrap_or_default()),
            id: jsonrpc_core::Id::Num(id),
        };

        let request_key = match MessageId::try_from(&request.id) {
            Ok(key) => key,
            Err(e) => return Err(ServerError::Request(format!("Invalid request ID: {}", e))),
        };
        let request_id = (self.conn_id.clone(), request_key.clone());

        let (response_tx, response_rx) = oneshot::channel();
        {
            let mut pending = self.pending_requests.lock().await;
            pending.insert(request_id.clone(), response_tx);
        }

        let conn_id = self.conn_id.clone();
        if let Err(e) = self.sender.send(request.into(), conn_id.clone()).await {
            let mut pending = self.pending_requests.lock().await;
            pending.remove(&(conn_id, request_key));
            return Err(ServerError::Transport(format!("Send: {}", e).into()));
        }

        let pending_requests = self.pending_requests.clone();
        let request_id_clone = request_id.clone();

        let future = async move {
            match tokio::time::timeout(timeout, response_rx).await {
                Ok(response) => match response {
                    Ok(result) => match result {
                        Ok(json_value) => serde_json::from_value(json_value)
                            .map_err(|e| anyhow::anyhow!("JSON deserialization error: {}", e)),
                        Err(server_error) => Err(anyhow::anyhow!("Server error: {}", server_error)),
                    },
                    Err(_) => Err(anyhow::anyhow!("Response channel closed")),
                },
                Err(_) => {
                    let mut pending = pending_requests.lock().await;
                    pending.remove(&request_id_clone);
                    Err(anyhow::anyhow!("Request timed out"))
                }
            }
        };

        Ok(Operation::new(request_id, future, self.sender.clone()))
    }

    pub async fn notify(&self, method: String, params: serde_json::Value) -> Result<(), ServerError> {
        let notification = jsonrpc_core::Notification {
            jsonrpc: Some(jsonrpc_core::Version::V2),
            method,
            params: Params::Map(params.as_object().cloned().unwrap_or_default()),
        };

        let conn_id = self.conn_id.clone();

        self.sender
            .send(notification.into(), conn_id)
            .await
            .map_err(|e| ServerError::Transport(format!("Send: {}", e).into()))
    }
}

pub struct Server<T: ModelContextProtocolServer> {
    server: Arc<RwLock<T>>,
    sessions: Arc<RwLock<HashMap<ConnectionId, Session>>>,
    pending_requests: PendingServerRequests,
    pending_client_requests: PendingClientRequests,
}

impl<T: ModelContextProtocolServer> Server<T> {
    pub fn new(server: T) -> Self {
        Server {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            server: Arc::new(RwLock::new(server)),
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            pending_client_requests: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn start(&self) -> Result<(), ServerError> {
        let transport_config = self.server.read().await.get_transport_config().await.clone();

        let (transport_type, mut on_client_rx, _on_error_rx, _on_close_rx) = match &transport_config {
            TransportConfig::Stdio(_config) => {
                let (on_message_tx, on_message_rx) = mpsc::channel::<Message>(32);
                let (on_error_tx, on_error_rx) = mpsc::channel(32);
                let (on_close_tx, on_close_rx) = mpsc::channel(32);

                let transport =
                    StdioTransport::new_server(on_message_tx.clone(), on_error_tx.clone(), on_close_tx.clone());
                (TransportType::Stdio(transport), on_message_rx, on_error_rx, on_close_rx)
            }
            TransportConfig::Sse(config) => {
                let (on_message_tx, on_message_rx) = mpsc::channel::<Message>(config.channel_capacity);
                let (on_error_tx, on_error_rx) = mpsc::channel(32);
                let (on_close_tx, on_close_rx) = mpsc::channel(32);

                let transport = SseTransport::new_server(
                    config.clone(),
                    on_message_tx.clone(),
                    on_error_tx.clone(),
                    on_close_tx.clone(),
                );
                (TransportType::Sse(transport), on_message_rx, on_error_rx, on_close_rx)
            }
            TransportConfig::Ws(config) => {
                let (on_message_tx, on_message_rx) = mpsc::channel::<Message>(32);
                let (on_error_tx, on_error_rx) = mpsc::channel(32);
                let (on_close_tx, on_close_rx) = mpsc::channel(32);

                let transport = WsTransport::new_server(
                    config.clone(),
                    on_message_tx.clone(),
                    on_error_tx.clone(),
                    on_close_tx.clone(),
                );
                (TransportType::Ws(transport), on_message_rx, on_error_rx, on_close_rx)
            }
        };

        let transport_sender = transport_type.sender();

        let mut io_handler = MetaIoHandler::default();

        let capabilities = self.server.read().await.get_capabilities().await.clone();
        let logging_enabled = capabilities.logging.is_some();

        let logging_layer = if logging_enabled {
            let layer = Arc::new(McpLoggingLayer::new(transport_sender.clone()));
            Some(layer)
        } else {
            None
        };

        self.setup_tracing(logging_enabled, logging_layer.clone()).await;

        io_handler.add_method_with_meta("initialize", {
            let server = self.server.clone();
            let sessions = self.sessions.clone();
            let transport_sender = transport_sender.clone();
            let pending_requests = self.pending_requests.clone();

            move |params: Params, meta: ServerMetadata| {
                let server = server.clone();
                let sessions = sessions.clone();
                let transport_sender = transport_sender.clone();
                let pending_requests = pending_requests.clone();

                debug!("Handling initialize request");

                async move {
                    let capabilities = server.read().await.get_capabilities().await.clone();

                    let init_params: InitializeRequestParams = params.parse().map_err(|e| {
                        error!("Failed to parse initialize parameters: {}", e);
                        jsonrpc_core::Error::invalid_params(e.to_string())
                    })?;

                    let result = InitializeResult {
                        capabilities: capabilities.clone(),
                        protocol_version: init_params.protocol_version,
                        server_info: Implementation {
                            name: "bioma-mcp-server".to_string(),
                            version: "0.1.0".to_string(),
                        },
                        instructions: Some("Bioma MCP server".to_string()),
                        meta: None,
                    };

                    let conn_id = meta.conn_id;
                    let pagination = server.read().await.get_pagination().await;

                    let context = Context {
                        conn_id: conn_id.clone(),
                        sender: transport_sender.clone(),
                        client_capabilities: init_params.capabilities.clone(),
                        server_capabilities: capabilities.clone(),
                        pending_requests: pending_requests.clone(),
                        request_counter: RequestCounter::default(),
                        pagination,
                    };

                    let server = server.read().await;
                    let tools = server.new_tools(context.clone()).await;
                    let resources = server.new_resources(context.clone()).await;
                    let prompts = server.new_prompts(context.clone()).await;

                    let session = Session { tools, resources, prompts, context };
                    sessions.write().await.insert(conn_id, session);

                    info!("Successfully handled initialize request");
                    Ok(serde_json::to_value(result).map_err(|e| {
                        error!("Failed to serialize initialize result: {}", e);
                        jsonrpc_core::Error::invalid_params(e.to_string())
                    })?)
                }
            }
        });

        io_handler.add_notification_with_meta("notifications/initialized", |params: Params, _meta: ServerMetadata| {
            match params.parse::<InitializedNotificationParams>() {
                Ok(_params) => {
                    info!("Received initialized notification");
                }
                Err(e) => {
                    error!("Failed to parse initialized notification params: {}", e);
                }
            }
        });

        io_handler.add_notification_with_meta("notifications/cancelled", {
            let pending_client_requests = self.pending_client_requests.clone();

            move |params: Params, meta: ServerMetadata| {
                let active_requests = pending_client_requests.clone();
                let conn_id = meta.conn_id.clone();

                let params_clone = params.clone();

                tokio::spawn(async move {
                    match params_clone.parse::<CancelledNotificationParams>() {
                        Ok(cancel_params) => {
                            let id = match &cancel_params.request_id {
                                serde_json::Value::Number(n) => {
                                    if let Some(num) = n.as_u64() {
                                        jsonrpc_core::Id::Num(num)
                                    } else {
                                        jsonrpc_core::Id::Null
                                    }
                                }
                                serde_json::Value::String(s) => jsonrpc_core::Id::Str(s.clone()),
                                _ => jsonrpc_core::Id::Null,
                            };

                            let request_key = match MessageId::try_from(&id) {
                                Ok(key) => key,
                                Err(e) => {
                                    error!("Invalid request ID in cancellation request: {}", e);
                                    return;
                                }
                            };

                            info!(
                                "Received cancellation for client request {} from connection {}: {}",
                                request_key,
                                conn_id,
                                cancel_params.reason.as_deref().unwrap_or("No reason provided")
                            );

                            let mut requests = active_requests.lock().await;
                            let key = (conn_id.clone(), request_key.clone());
                            if let Some(abort_handle) = requests.remove(&key) {
                                abort_handle.abort();
                                info!(
                                    "Successfully aborted processing for request {} from connection {}",
                                    request_key, conn_id
                                );
                            } else {
                                debug!(
                                    "Request {} from connection {} not found or already completed",
                                    request_key, conn_id
                                );
                            }
                        }
                        Err(e) => {
                            error!("Failed to parse cancellation params: {}", e);
                        }
                    }
                });

                match params.parse::<CancelledNotificationParams>() {
                    Ok(cancel_params) => {
                        let id = match &cancel_params.request_id {
                            serde_json::Value::Number(n) => {
                                if let Some(num) = n.as_u64() {
                                    jsonrpc_core::Id::Num(num)
                                } else {
                                    error!("Invalid numeric ID in cancellation request");
                                    return;
                                }
                            }
                            serde_json::Value::String(s) => jsonrpc_core::Id::Str(s.clone()),
                            _ => {
                                error!("Unsupported ID type in cancellation request");
                                return;
                            }
                        };

                        let request_key = match MessageId::try_from(&id) {
                            Ok(key) => key,
                            Err(e) => {
                                error!("Invalid request ID in cancellation request: {}", e);
                                return;
                            }
                        };
                        debug!(
                            "Received cancellation notification for request ID: {} from connection {}",
                            request_key, meta.conn_id
                        );
                    }
                    Err(e) => {
                        error!("Failed to parse cancellation params (sync): {}", e);
                    }
                }
            }
        });

        io_handler.add_method_with_meta("ping", move |params: Params, _meta: ServerMetadata| {
            debug!("Handling ping request");

            async move {
                let _params: PingRequestParams = match params.parse() {
                    Ok(params) => params,
                    Err(e) => {
                        error!("Failed to parse ping parameters: {}", e);
                        return Err(jsonrpc_core::Error::invalid_params(e.to_string()));
                    }
                };

                info!("Successfully handled ping request");
                Ok(serde_json::json!({
                    "result": "pong",
                }))
            }
        });

        io_handler.add_method_with_meta("resources/list", {
            let sessions = self.sessions.clone();

            move |params: Params, meta: ServerMetadata| {
                let sessions = sessions.clone();

                debug!("Handling resources/list request");

                async move {
                    let params: ListResourcesRequestParams = match params.parse() {
                        Ok(params) => params,
                        Err(e) => {
                            error!("Failed to parse resources/list parameters: {}", e);
                            return Err(jsonrpc_core::Error::invalid_params(e.to_string()));
                        }
                    };

                    debug!("Resources list request with cursor: {:?}", params.cursor);

                    let sessions = sessions.read().await;
                    let Some(session) = sessions.get(&meta.conn_id) else {
                        error!("Session not found");
                        return Err(jsonrpc_core::Error::invalid_params("Session not found".to_string()));
                    };

                    let (resources, next_cursor) = session.list_resources(params.cursor);
                    let response = ListResourcesResult { next_cursor, resources, meta: None };

                    info!("Successfully handled resources/list request");
                    Ok(serde_json::to_value(response).unwrap_or_default())
                }
            }
        });

        io_handler.add_method_with_meta("resources/read", {
            let sessions = self.sessions.clone();
            move |params: Params, meta: ServerMetadata| {
                let sessions = sessions.clone();
                async move {
                    debug!("Handling resources/read request");

                    let params: ReadResourceRequestParams = match params.parse() {
                        Ok(params) => params,
                        Err(e) => {
                            error!("Failed to parse resources/read parameters: {}", e);
                            return Err(jsonrpc_core::Error::invalid_params(e.to_string()));
                        }
                    };

                    debug!("Requested resource URI: {}", params.uri);

                    let sessions = sessions.read().await;
                    let Some(session) = sessions.get(&meta.conn_id) else {
                        error!("Session not found");
                        return Err(jsonrpc_core::Error::invalid_params("Session not found".to_string()));
                    };

                    let resource = session.find_resource(&params.uri);

                    match resource {
                        Some(resource) => match resource.read_boxed(params.uri.clone()).await {
                            Ok(result) => {
                                info!("Successfully handled resources/read request for: {}", params.uri);
                                Ok(serde_json::to_value(result).map_err(|e| {
                                    error!("Failed to serialize resources/read result: {}", e);
                                    jsonrpc_core::Error::internal_error()
                                })?)
                            }
                            Err(e) => {
                                error!("Failed to read resource: {}", e);
                                Err(jsonrpc_core::Error::internal_error())
                            }
                        },
                        None => {
                            error!("Resource not found: {}", params.uri);
                            Err(jsonrpc_core::Error::invalid_params(format!("Resource not found: {}", params.uri)))
                        }
                    }
                }
            }
        });

        io_handler.add_method_with_meta("resources/templates/list", {
            let sessions = self.sessions.clone();
            move |params: Params, meta: ServerMetadata| {
                let sessions = sessions.clone();
                async move {
                    debug!("Handling resources/templates/list request");

                    let params: ListResourceTemplatesRequestParams = match params.parse() {
                        Ok(params) => params,
                        Err(e) => {
                            error!("Failed to parse resources/templates/list parameters: {}", e);
                            return Err(jsonrpc_core::Error::invalid_params(e.to_string()));
                        }
                    };

                    debug!("Resource templates list request with cursor: {:?}", params.cursor);

                    let sessions = sessions.read().await;
                    let Some(session) = sessions.get(&meta.conn_id) else {
                        error!("Session not found");
                        return Err(jsonrpc_core::Error::invalid_params("Session not found".to_string()));
                    };

                    let resource_templates =
                        session.resources.iter().map(|resource| resource.templates()).flatten().collect::<Vec<_>>();

                    let response = ListResourceTemplatesResult { next_cursor: None, resource_templates, meta: None };

                    info!(
                        "Successfully handled resources/templates/list request, found {} templates",
                        response.resource_templates.len()
                    );
                    Ok(serde_json::to_value(response).unwrap_or_default())
                }
            }
        });

        io_handler.add_method_with_meta("resources/subscribe", {
            let sessions = self.sessions.clone();

            move |params: Params, meta: ServerMetadata| {
                let sessions = sessions.clone();
                let conn_id = meta.conn_id.clone();

                async move {
                    debug!("Handling resources/subscribe request");

                    let params: SubscribeRequestParams = match params.parse() {
                        Ok(params) => params,
                        Err(e) => {
                            error!("Failed to parse resources/subscribe parameters: {}", e);
                            return Err(jsonrpc_core::Error::invalid_params(e.to_string()));
                        }
                    };

                    debug!("Subscribe resource URI: {}", params.uri);

                    let sessions = sessions.read().await;
                    let Some(session) = sessions.get(&conn_id) else {
                        error!("Session not found");
                        return Err(jsonrpc_core::Error::invalid_params("Session not found".to_string()));
                    };

                    let resource = session
                        .resources
                        .iter()
                        .find(|resource| resource.supports(&params.uri) && resource.supports_subscription(&params.uri));

                    match resource {
                        Some(resource) => match resource.subscribe(params.uri.clone()).await {
                            Ok(_) => {
                                info!("Successfully subscribed to resource: {} for client: {}", params.uri, conn_id);
                                Ok(serde_json::json!({}))
                            }
                            Err(e) => {
                                error!("Failed to subscribe to resource: {}", e);
                                Err(jsonrpc_core::Error::internal_error())
                            }
                        },
                        None => {
                            error!("Resource not found or doesn't support subscription: {}", params.uri);
                            Err(jsonrpc_core::Error::invalid_params(format!(
                                "Resource not found or doesn't support subscription: {}",
                                params.uri
                            )))
                        }
                    }
                }
            }
        });

        io_handler.add_method_with_meta("resources/unsubscribe", {
            let sessions = self.sessions.clone();
            move |params: Params, meta: ServerMetadata| {
                let sessions = sessions.clone();
                let conn_id = meta.conn_id.clone();

                async move {
                    debug!("Handling resources/unsubscribe request");

                    let params: UnsubscribeRequestParams = match params.parse() {
                        Ok(params) => params,
                        Err(e) => {
                            error!("Failed to parse resources/unsubscribe parameters: {}", e);
                            return Err(jsonrpc_core::Error::invalid_params(e.to_string()));
                        }
                    };

                    debug!("Unsubscribe resource URI: {}", params.uri);

                    let sessions = sessions.read().await;
                    let Some(session) = sessions.get(&conn_id) else {
                        error!("Session not found");
                        return Err(jsonrpc_core::Error::invalid_params("Session not found".to_string()));
                    };

                    let resource = session
                        .resources
                        .iter()
                        .find(|resource| resource.supports(&params.uri) && resource.supports_subscription(&params.uri));

                    match resource {
                        Some(resource) => match resource.unsubscribe(params.uri.clone()).await {
                            Ok(_) => {
                                info!("Successfully unsubscribed from resource: {}", params.uri);
                                Ok(serde_json::json!({}))
                            }
                            Err(e) => {
                                error!("Failed to unsubscribe from resource: {}", e);
                                Err(jsonrpc_core::Error::internal_error())
                            }
                        },
                        None => {
                            error!("Resource not found or doesn't support subscription: {}", params.uri);
                            Err(jsonrpc_core::Error::invalid_params(format!(
                                "Resource not found or doesn't support subscription: {}",
                                params.uri
                            )))
                        }
                    }
                }
            }
        });

        io_handler.add_method_with_meta("prompts/list", {
            let sessions = self.sessions.clone();
            move |params: Params, meta: ServerMetadata| {
                let sessions = sessions.clone();
                debug!("Handling prompts/list request");

                async move {
                    let params: ListPromptsRequestParams = match params.parse() {
                        Ok(params) => params,
                        Err(e) => {
                            error!("Failed to parse prompts/list parameters: {}", e);
                            return Err(jsonrpc_core::Error::invalid_params(e.to_string()));
                        }
                    };

                    debug!("Prompts list request with cursor: {:?}", params.cursor);

                    let sessions = sessions.read().await;
                    let Some(session) = sessions.get(&meta.conn_id) else {
                        error!("Session not found");
                        return Err(jsonrpc_core::Error::invalid_params("Session not found".to_string()));
                    };

                    let (prompts, next_cursor) = session.list_prompts(params.cursor);
                    let response = ListPromptsResult { next_cursor, prompts, meta: None };

                    info!("Successfully handled prompts/list request");
                    Ok(serde_json::to_value(response).unwrap_or_default())
                }
            }
        });

        io_handler.add_method_with_meta("prompts/get", {
            let sessions = self.sessions.clone();
            move |params: Params, meta: ServerMetadata| {
                let sessions = sessions.clone();
                let conn_id = meta.conn_id.clone();

                debug!("Handling prompts/get request");

                async move {
                    let params: GetPromptRequestParams = params.parse().map_err(|e| {
                        error!("Failed to parse prompt get parameters: {}", e);
                        jsonrpc_core::Error::invalid_params(e.to_string())
                    })?;

                    let sessions = sessions.read().await;
                    let Some(session) = sessions.get(&conn_id) else {
                        error!("Session not found");
                        return Err(jsonrpc_core::Error::invalid_params("Session not found".to_string()));
                    };

                    let prompt = session.find_prompt(&params.name);

                    match prompt {
                        Some(prompt) => {
                            let result = prompt.get_boxed(params.arguments).await.map_err(|e| {
                                error!("Prompt execution failed: {}", e);
                                jsonrpc_core::Error::internal_error()
                            })?;

                            info!("Successfully handled prompts/get request for: {}", params.name);
                            Ok(serde_json::to_value(result).map_err(|e| {
                                error!("Failed to serialize prompt result: {}", e);
                                jsonrpc_core::Error::invalid_params(e.to_string())
                            })?)
                        }
                        None => {
                            error!("Unknown prompt requested: {}", params.name);
                            Err(jsonrpc_core::Error::method_not_found())
                        }
                    }
                }
            }
        });

        io_handler.add_method_with_meta("tools/list", {
            let sessions = self.sessions.clone();

            move |params: Params, meta: ServerMetadata| {
                let sessions = sessions.clone();

                debug!("Handling tools/list request");

                async move {
                    let params: ListToolsRequestParams = match params.parse() {
                        Ok(params) => params,
                        Err(e) => {
                            error!("Failed to parse tools/list parameters: {}", e);
                            return Err(jsonrpc_core::Error::invalid_params(e.to_string()));
                        }
                    };

                    debug!("Tools list request with cursor: {:?}", params.cursor);

                    let conn_id = meta.conn_id.clone();
                    let sessions = sessions.read().await;

                    let (tools, next_cursor) = if let Some(session) = sessions.get(&conn_id) {
                        session.list_tools(params.cursor)
                    } else {
                        (vec![], None)
                    };
                    let response = ListToolsResult { next_cursor, tools, meta: None };
                    info!("Successfully handled tools/list request");
                    Ok(serde_json::to_value(response).unwrap_or_default())
                }
            }
        });

        io_handler.add_method_with_meta("tools/call", {
            let sessions = self.sessions.clone();

            move |params: Params, meta: ServerMetadata| {
                let sessions = sessions.clone();

                debug!("Handling tools/call request");

                async move {
                    let params: CallToolRequestParams = params.parse().map_err(|e| {
                        error!("Failed to parse tool call parameters: {}", e);
                        jsonrpc_core::Error::invalid_params(e.to_string())
                    })?;

                    let tool_reference = {
                        let sessions = sessions.read().await;
                        if let Some(session) = sessions.get(&meta.conn_id) {
                            let tool_reference = session.find_tool(&params.name);
                            tool_reference
                        } else {
                            None
                        }
                    };

                    match tool_reference {
                        Some(tool) => {
                            let result = tool.call_boxed(params.arguments).await.map_err(|e| {
                                error!("Tool execution failed: {}", e);
                                jsonrpc_core::Error::internal_error()
                            })?;

                            info!("Successfully handled tool call for: {}", params.name);
                            Ok(serde_json::to_value(result).map_err(|e| {
                                error!("Failed to serialize tool call result: {}", e);
                                jsonrpc_core::Error::invalid_params(e.to_string())
                            })?)
                        }
                        None => {
                            error!("Unknown tool requested: {}", params.name);
                            Err(jsonrpc_core::Error::method_not_found())
                        }
                    }
                }
            }
        });

        io_handler.add_method_with_meta("completion/complete", {
            let sessions = self.sessions.clone();

            move |params: Params, meta: ServerMetadata| {
                let sessions = sessions.clone();

                async move {
                    debug!("Handling completion/complete request");

                    let params: crate::schema::CompleteRequestParams = params.parse().map_err(|e| {
                        error!("Failed to parse completion params: {}", e);
                        jsonrpc_core::Error::invalid_params(e.to_string())
                    })?;

                    let sessions = sessions.read().await;
                    let Some(session) = sessions.get(&meta.conn_id) else {
                        error!("Session not found");
                        return Err(jsonrpc_core::Error::invalid_params("Session not found".to_string()));
                    };

                    match &params.ref_ {
                        serde_json::Value::Object(obj) => {
                            if let Some(type_val) = obj.get("type") {
                                if let Some(type_str) = type_val.as_str() {
                                    if type_str == "ref/prompt" {
                                        if let Some(name_val) = obj.get("name") {
                                            if let Some(name_str) = name_val.as_str() {
                                                if let Some(prompt) = session.find_prompt(name_str) {
                                                    let completions = prompt
                                                        .complete_argument_boxed(
                                                            params.argument.name.clone(),
                                                            params.argument.value.clone(),
                                                        )
                                                        .await
                                                        .map_err(|e| {
                                                            error!("Prompt completion error: {}", e);
                                                            jsonrpc_core::Error::internal_error()
                                                        })?;

                                                    return Ok(serde_json::to_value(crate::schema::CompleteResult {
                                                        meta: None,
                                                        completion: crate::schema::CompleteResultCompletion {
                                                            values: completions,
                                                            total: None,
                                                            has_more: Some(false),
                                                        },
                                                    })
                                                    .map_err(|e| {
                                                        error!("Failed to serialize completion result: {}", e);
                                                        jsonrpc_core::Error::internal_error()
                                                    })?);
                                                }
                                            }
                                        }
                                    } else if type_str == "ref/resource" {
                                        if let Some(uri_val) = obj.get("uri") {
                                            if let Some(uri_str) = uri_val.as_str() {
                                                if let Some(resource) = session.find_resource(uri_str) {
                                                    let completions = resource
                                                        .complete_boxed(
                                                            params.argument.name.clone(),
                                                            params.argument.value.clone(),
                                                        )
                                                        .await
                                                        .map_err(|e| {
                                                            error!("Resource completion error: {}", e);
                                                            jsonrpc_core::Error::internal_error()
                                                        })?;

                                                    return Ok(serde_json::to_value(crate::schema::CompleteResult {
                                                        meta: None,
                                                        completion: crate::schema::CompleteResultCompletion {
                                                            values: completions,
                                                            total: None,
                                                            has_more: Some(false),
                                                        },
                                                    })
                                                    .map_err(|e| {
                                                        error!("Failed to serialize completion result: {}", e);
                                                        jsonrpc_core::Error::internal_error()
                                                    })?);
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            error!("Invalid reference format");
                            Err(jsonrpc_core::Error::invalid_params("Invalid reference format"))
                        }
                        _ => {
                            error!("Invalid reference type");
                            Err(jsonrpc_core::Error::invalid_params("Invalid reference type"))
                        }
                    }
                }
            }
        });

        if logging_enabled {
            if let Some(layer) = &logging_layer {
                let logging_layer_clone = layer.clone();

                io_handler.add_method_with_meta("logging/setLevel", move |params: Params, meta: ServerMetadata| {
                    let logging_layer = logging_layer_clone.clone();
                    let conn_id = meta.conn_id.clone();

                    async move {
                        let params: crate::schema::SetLevelRequestParams = params.parse().map_err(|e| {
                            let msg = format!("Failed to parse logging/setLevel parameters: {}", e);
                            jsonrpc_core::Error::invalid_params(msg)
                        })?;

                        logging_layer.set_client_level(conn_id, params.level);

                        Ok(serde_json::json!({}))
                    }
                });
            }
        }

        let transport = Arc::new(Mutex::new(transport_type));

        {
            let mut transport_lock = transport.lock().await;
            if let Err(e) = transport_lock.start().await {
                error!("Transport error: {}", e);
                return Err(ServerError::Transport(e.to_string()));
            }
        }

        let pending_requests = self.pending_requests.clone();

        while let Some(message) = on_client_rx.recv().await {
            let io_handler_clone = io_handler.clone();
            let transport_sender_clone = transport_sender.clone();
            let pending_requests = pending_requests.clone();
            let pending_client_requests_clone = self.pending_client_requests.clone();

            tokio::spawn(async move {
                match &message.message {
                    JsonRpcMessage::Request(request) => match request {
                        jsonrpc_core::Request::Single(jsonrpc_core::Call::MethodCall(call)) => {
                            let metadata = ServerMetadata { conn_id: message.conn_id.clone() };
                            let request_key = match MessageId::try_from(&call.id) {
                                Ok(key) => key,
                                Err(e) => {
                                    error!("Invalid request ID: {}", e);
                                    return;
                                }
                            };

                            let request_key_clone = request_key.clone();
                            let conn_id_clone = message.conn_id.clone();

                            let abort_handle = {
                                let request_clone = request.clone();
                                let message_conn_id = message.conn_id.clone();
                                let io_handler_clone_inner = io_handler_clone.clone();
                                let transport_sender_clone_inner = transport_sender_clone.clone();
                                let active_requests_inner = pending_client_requests_clone.clone();

                                let handle = tokio::spawn(async move {
                                    if let Some(response) =
                                        io_handler_clone_inner.handle_rpc_request(request_clone, metadata).await
                                    {
                                        if let Err(e) = transport_sender_clone_inner
                                            .send(response.into(), message_conn_id.clone())
                                            .await
                                        {
                                            error!("Failed to send response: {}", e);
                                        }
                                    }

                                    let mut requests = active_requests_inner.lock().await;
                                    requests.remove(&(message_conn_id.clone(), request_key_clone));
                                });

                                handle.abort_handle()
                            };

                            let mut active_requests = pending_client_requests_clone.lock().await;
                            active_requests.insert((conn_id_clone, request_key), abort_handle);
                        }
                        jsonrpc_core::Request::Single(jsonrpc_core::Call::Notification(notification)) => {
                            let metadata = ServerMetadata { conn_id: message.conn_id.clone() };
                            let request_clone =
                                jsonrpc_core::Request::Single(jsonrpc_core::Call::Notification(notification.clone()));

                            if let Some(result) = io_handler_clone.handle_rpc_request(request_clone, metadata).await {
                                debug!("Notification handled successfully {:?}", result);
                            }
                        }
                        _ => {
                            warn!("Unsupported batch request: {:?}", request);
                        }
                    },
                    JsonRpcMessage::Response(response) => match response {
                        jsonrpc_core::Response::Single(output) => match output {
                            jsonrpc_core::Output::Success(success) => {
                                let request_key = match MessageId::try_from(&success.id) {
                                    Ok(key) => key,
                                    Err(e) => {
                                        error!("Invalid request ID: {}", e);
                                        return;
                                    }
                                };
                                let mut requests = pending_requests.lock().await;
                                let request_id = (message.conn_id.clone(), request_key);
                                if let Some(sender) = requests.remove(&request_id) {
                                    let _ = sender.send(Ok(success.result.clone()));
                                }
                            }
                            jsonrpc_core::Output::Failure(failure) => {
                                let request_key = match MessageId::try_from(&failure.id) {
                                    Ok(key) => key,
                                    Err(e) => {
                                        error!("Invalid request ID: {}", e);
                                        return;
                                    }
                                };
                                let mut requests = pending_requests.lock().await;
                                let request_id = (message.conn_id.clone(), request_key);
                                if let Some(sender) = requests.remove(&request_id) {
                                    let _ = sender
                                        .send(Err(ServerError::Request(format!("RPC error: {:?}", failure.error))));
                                }
                            }
                        },
                        jsonrpc_core::Response::Batch(_) => {
                            warn!("Unsupported batch response");
                        }
                    },
                }
            });
        }

        Ok(())
    }

    async fn setup_tracing(&self, logging_enabled: bool, mcp_layer: Option<Arc<McpLoggingLayer>>) {
        let base_layer_result = self.server.read().await.get_tracing_layer().await;

        let filter = std::env::var("RUST_LOG")
            .map(|val| tracing_subscriber::EnvFilter::new(val))
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug"));

        match (logging_enabled, base_layer_result, mcp_layer) {
            (true, Some(base_layer), Some(mcp_layer)) => {
                debug!("Setting up tracing with base layer and MCP layer");
                let mcp_layer = (*mcp_layer).clone().with_filter(filter);
                let _ = tracing_subscriber::registry()
                    .with(base_layer)
                    .with(mcp_layer)
                    .try_init()
                    .map_err(|e| debug!("Could not initialize tracing with both layers: {}", e));
            }
            (true, None, Some(mcp_layer)) => {
                debug!("Setting up tracing with MCP layer only");
                let mcp_layer = (*mcp_layer).clone().with_filter(filter);
                let _ = tracing_subscriber::registry()
                    .with(mcp_layer)
                    .try_init()
                    .map_err(|e| debug!("Could not initialize tracing with MCP layer: {}", e));
            }
            (_, Some(base_layer), _) => {
                debug!("Setting up tracing with base layer only");
                let _ = tracing_subscriber::registry()
                    .with(base_layer)
                    .try_init()
                    .map_err(|e| debug!("Could not initialize tracing with base layer: {}", e));
            }
            _ => {
                debug!("No tracing layers provided or enabled");
            }
        }
    }
}
