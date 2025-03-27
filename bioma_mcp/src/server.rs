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
use crate::{ConnectionId, JsonRpcMessage};
use jsonrpc_core::{MetaIoHandler, Metadata, Params};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tracing::{debug, error, info, warn};

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

pub trait ModelContextProtocolServer: Send + Sync + 'static {
    fn get_transport_config(&self) -> impl Future<Output = TransportConfig> + Send;
    fn get_capabilities(&self) -> impl Future<Output = ServerCapabilities> + Send;
    fn new_resources(&self, context: Context) -> impl Future<Output = Vec<Arc<dyn ResourceReadHandler>>> + Send;
    fn new_prompts(&self, context: Context) -> impl Future<Output = Vec<Arc<dyn PromptGetHandler>>> + Send;
    fn new_tools(&self, context: Context) -> impl Future<Output = Vec<Arc<dyn ToolCallHandler>>> + Send;
    fn on_error(&self, error: anyhow::Error) -> impl Future<Output = ()> + Send;
    fn get_pagination_config(&self) -> impl Future<Output = crate::pagination::PaginationConfig> + Send {
        async { crate::pagination::PaginationConfig::default() }
    }
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

    fn list_tools(&self) -> Vec<crate::schema::Tool> {
        self.tools.iter().map(|tool| tool.def()).collect()
    }

    fn find_resource(&self, uri: &str) -> Option<Arc<dyn ResourceReadHandler>> {
        self.resources.iter().find(|resource| resource.def().uri == uri).cloned()
    }

    fn list_resources(&self) -> Vec<crate::schema::Resource> {
        self.resources.iter().map(|resource| resource.def()).collect()
    }

    fn find_prompt(&self, name: &str) -> Option<Arc<dyn PromptGetHandler>> {
        self.prompts.iter().find(|prompt| prompt.def().name == name).cloned()
    }

    fn list_prompts(&self) -> Vec<crate::schema::Prompt> {
        self.prompts.iter().map(|prompt| prompt.def()).collect()
    }
}

type RequestId = u64;
type ResponseSender = oneshot::Sender<Result<serde_json::Value, ServerError>>;
type PendingRequests = Arc<Mutex<HashMap<RequestId, ResponseSender>>>;
type RequestCounter = Arc<RwLock<u64>>;

#[derive(Clone)]
pub struct Context {
    pub client_capabilities: ClientCapabilities,
    pub server_capabilities: ServerCapabilities,
    conn_id: ConnectionId,
    sender: TransportSender,
    pending_requests: PendingRequests,
    request_counter: RequestCounter,
}

impl Context {
    pub fn test() -> Self {
        Self {
            client_capabilities: ClientCapabilities::default(),
            server_capabilities: ServerCapabilities::default(),
            conn_id: ConnectionId(uuid::Uuid::new_v4()),
            sender: TransportSender::new_nop(),
            pending_requests: PendingRequests::default(),
            request_counter: RequestCounter::default(),
        }
    }

    pub async fn create_message(&self, params: CreateMessageRequestParams) -> Result<CreateMessageResult, ServerError> {
        let params = serde_json::to_value(params).unwrap_or_default();
        let result =
            self.request("sampling/createMessage".to_string(), params, std::time::Duration::from_secs(10)).await?;
        Ok(serde_json::from_value(result).map_err(|e| ServerError::ParseResponse(e.to_string()))?)
    }

    pub async fn resource_updated(&self, params: ResourceUpdatedNotificationParams) -> Result<(), ServerError> {
        let params = serde_json::to_value(params).unwrap_or_default();
        self.notify("notifications/resources/updated".to_string(), params).await?;
        Ok(())
    }

    pub async fn request(
        &self,
        method: String,
        params: serde_json::Value,
        timeout: std::time::Duration,
    ) -> Result<serde_json::Value, ServerError> {
        let mut counter = self.request_counter.write().await;
        *counter += 1;
        let id = *counter;

        let request = jsonrpc_core::MethodCall {
            jsonrpc: Some(jsonrpc_core::Version::V2),
            method,
            params: Params::Map(params.as_object().cloned().unwrap_or_default()),
            id: jsonrpc_core::Id::Num(id),
        };

        let (response_tx, response_rx) = oneshot::channel();

        {
            let mut pending = self.pending_requests.lock().await;
            pending.insert(id, response_tx);
        }

        let conn_id = self.conn_id.clone();

        if let Err(e) = self.sender.send(request.into(), conn_id).await {
            let mut pending = self.pending_requests.lock().await;
            pending.remove(&id);
            return Err(ServerError::Transport(format!("Send: {}", e).into()));
        }

        match tokio::time::timeout(timeout, response_rx).await {
            Ok(response) => match response {
                Ok(result) => result,
                Err(_) => Err(ServerError::Request("Response channel closed".into())),
            },
            Err(_) => {
                let mut pending = self.pending_requests.lock().await;
                pending.remove(&id);
                Err(ServerError::Request("Request timed out".into()))
            }
        }
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
    pending_requests: PendingRequests,
    request_counter: RequestCounter,
}

impl<T: ModelContextProtocolServer> Server<T> {
    pub fn new(server: T) -> Self {
        Server {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            server: Arc::new(RwLock::new(server)),
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            request_counter: Arc::new(RwLock::new(0)),
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

        let transport = Arc::new(Mutex::new(transport_type));

        let mut io_handler = MetaIoHandler::default();

        io_handler.add_method_with_meta("initialize", {
            let server = self.server.clone();
            let sessions = self.sessions.clone();
            let transport_sender = transport_sender.clone();
            let pending_requests = self.pending_requests.clone();
            let request_counter = self.request_counter.clone();

            move |params: Params, meta: ServerMetadata| {
                let server = server.clone();
                let sessions = sessions.clone();
                let transport_sender = transport_sender.clone();
                let pending_requests = pending_requests.clone();
                let request_counter = request_counter.clone();

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

                    let context = Context {
                        conn_id: conn_id.clone(),
                        sender: transport_sender.clone(),
                        client_capabilities: init_params.capabilities.clone(),
                        server_capabilities: capabilities.clone(),
                        pending_requests: pending_requests.clone(),
                        request_counter: request_counter.clone(),
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

        io_handler.add_notification_with_meta("cancelled", move |params: Params, _meta: ServerMetadata| {
            match params.parse::<CancelledNotificationParams>() {
                Ok(cancel_params) => {
                    info!(
                        "Received cancellation for request {}: {}",
                        cancel_params.request_id,
                        cancel_params.reason.unwrap_or_default()
                    );
                }
                Err(e) => {
                    error!("Failed to parse cancellation params: {}", e);
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
            let server = self.server.clone();

            move |params: Params, meta: ServerMetadata| {
                let sessions = sessions.clone();
                let server = server.clone();

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

                    let all_resources = session.list_resources();
                    
                    if !crate::pagination::validate_cursor(params.cursor.as_deref()) {
                        error!("Invalid cursor provided: {:?}", params.cursor);
                        return Err(jsonrpc_core::Error::invalid_params("Invalid cursor".to_string()));
                    }
                    
                    let pagination_config = server.read().await.get_pagination_config().await;
                    
                    let (resources, next_cursor) = crate::pagination::paginate(
                        &all_resources,
                        params.cursor.as_deref(),
                        &pagination_config,
                    );
                    
                    let response = ListResourcesResult { next_cursor, resources: resources.clone(), meta: None };

                    info!("Successfully handled resources/list request, returned {} of {} resources with page size {}", 
                        resources.len(), all_resources.len(), pagination_config.page_size);
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
            let server = self.server.clone();
            
            move |params: Params, meta: ServerMetadata| {
                let sessions = sessions.clone();
                let server = server.clone();
                
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

                    let all_templates = session.resources.iter()
                        .flat_map(|resource| resource.templates())
                        .collect::<Vec<_>>();

                    if !crate::pagination::validate_cursor(params.cursor.as_deref()) {
                        error!("Invalid cursor provided: {:?}", params.cursor);
                        return Err(jsonrpc_core::Error::invalid_params("Invalid cursor".to_string()));
                    }

                    let pagination_config = server.read().await.get_pagination_config().await;
                    
                    let (resource_templates, next_cursor) = crate::pagination::paginate(
                        &all_templates,
                        params.cursor.as_deref(),
                        &pagination_config,
                    );

                    let response = ListResourceTemplatesResult { next_cursor, resource_templates: resource_templates.clone(), meta: None };

                    info!(
                        "Successfully handled resources/templates/list request, returned {} of {} templates with page size {}",
                        resource_templates.len(),
                        all_templates.len(),
                        pagination_config.page_size
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
            let server = self.server.clone();
            
            move |params: Params, meta: ServerMetadata| {
                let sessions = sessions.clone();
                let server = server.clone();
                
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

                    let all_prompts = session.list_prompts();

                    if !crate::pagination::validate_cursor(params.cursor.as_deref()) {
                        error!("Invalid cursor provided: {:?}", params.cursor);
                        return Err(jsonrpc_core::Error::invalid_params("Invalid cursor".to_string()));
                    }

                    let pagination_config = server.read().await.get_pagination_config().await;
                    
                    let (prompts, next_cursor) = crate::pagination::paginate(
                        &all_prompts,
                        params.cursor.as_deref(),
                        &pagination_config,
                    );

                    let response = ListPromptsResult { next_cursor, prompts: prompts.clone(), meta: None };

                    info!("Successfully handled prompts/list request, returned {} of {} prompts with page size {}", 
                        prompts.len(), all_prompts.len(), pagination_config.page_size);
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
            let server = self.server.clone();

            move |params: Params, meta: ServerMetadata| {
                let sessions = sessions.clone();
                let server = server.clone();

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

                    let all_tools = if let Some(session) = sessions.get(&conn_id) { 
                        session.list_tools() 
                    } else { 
                        vec![] 
                    };
                    
                    if !crate::pagination::validate_cursor(params.cursor.as_deref()) {
                        error!("Invalid cursor provided: {:?}", params.cursor);
                        return Err(jsonrpc_core::Error::invalid_params("Invalid cursor".to_string()));
                    }
                    
                    let pagination_config = server.read().await.get_pagination_config().await;
                    
                    let (tools, next_cursor) = crate::pagination::paginate(
                        &all_tools,
                        params.cursor.as_deref(),
                        &pagination_config,
                    );
                    
                    let response = ListToolsResult { next_cursor, tools: tools.clone(), meta: None };
                    
                    info!("Successfully handled tools/list request, returned {} of {} tools with page size {}", 
                        tools.len(), all_tools.len(), pagination_config.page_size);
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

            tokio::spawn(async move {
                match &message.message {
                    JsonRpcMessage::Request(request) => match request {
                        jsonrpc_core::Request::Single(jsonrpc_core::Call::MethodCall(_call)) => {
                            let metadata = ServerMetadata { conn_id: message.conn_id.clone() };

                            let Some(response) = io_handler_clone.handle_rpc_request(request.clone(), metadata).await
                            else {
                                return;
                            };

                            if let Err(e) = transport_sender_clone.send(response.into(), message.conn_id.clone()).await
                            {
                                error!("Failed to send response: {}", e);
                            }
                        }
                        jsonrpc_core::Request::Single(jsonrpc_core::Call::Notification(notification)) => {
                            debug!("Handled notification: {:?}", notification.method);
                        }
                        _ => {
                            warn!("Unsupported batch request: {:?}", request);
                        }
                    },
                    JsonRpcMessage::Response(response) => match response {
                        jsonrpc_core::Response::Single(output) => match output {
                            jsonrpc_core::Output::Success(success) => {
                                if let jsonrpc_core::Id::Num(id) = success.id {
                                    let mut requests = pending_requests.lock().await;
                                    if let Some(sender) = requests.remove(&id) {
                                        let _ = sender.send(Ok(success.result.clone()));
                                    }
                                }
                            }
                            jsonrpc_core::Output::Failure(failure) => {
                                if let jsonrpc_core::Id::Num(id) = failure.id {
                                    let mut requests = pending_requests.lock().await;
                                    if let Some(sender) = requests.remove(&id) {
                                        let _ = sender
                                            .send(Err(ServerError::Request(format!("RPC error: {:?}", failure.error))));
                                    }
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
}
