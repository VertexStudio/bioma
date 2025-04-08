use crate::operation::Operation;
use crate::schema::{
    CallToolRequestParams, CallToolResult, CancelledNotificationParams, ClientCapabilities, CompleteRequestParams,
    CompleteRequestParamsArgument, CompleteResult, CompleteResultCompletion, CreateMessageRequestParams,
    CreateMessageResult, GetPromptRequestParams, GetPromptResult, Implementation, InitializeRequestParams,
    InitializeResult, InitializedNotificationParams, ListPromptsRequestParams, ListPromptsResult,
    ListResourceTemplatesRequestParams, ListResourceTemplatesResult, ListResourcesRequestParams, ListResourcesResult,
    ListToolsRequestParams, ListToolsResult, LoggingLevel, LoggingMessageNotificationParams,
    ProgressNotificationParams, ProgressToken, Prompt, PromptReference, ReadResourceRequestParams, ReadResourceResult,
    Resource, ResourceReference, ResourceTemplate, Root, RootsListChangedNotificationParams, ServerCapabilities, Tool,
};
use crate::transport::sse::SseTransport;
use crate::transport::ws::WsTransport;
use crate::transport::{stdio::StdioTransport, Transport, TransportSender, TransportType};
use crate::{ConnectionId, JsonRpcMessage, MessageId, RequestId};
use anyhow::Error;
use base64;
use jsonrpc_core::{MetaIoHandler, Metadata, Params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::task::AbortHandle;
use tracing::{debug, error, info, warn};
use uuid;

#[derive(Clone)]
pub struct ClientMetadata {
    pub conn_id: ConnectionId,
}

impl Metadata for ClientMetadata {}

#[derive(Serialize, Deserialize)]
struct MultiServerCursor {
    server_cursors: HashMap<String, Option<String>>,
}

impl MultiServerCursor {
    fn to_string(&self) -> Result<String, ClientError> {
        let encoded = serde_json::to_string(self)
            .map_err(|e| ClientError::Request(format!("Failed to encode cursor: {}", e).into()))?;
        Ok(base64::encode(encoded))
    }

    fn from_string(s: &str) -> Result<Self, ClientError> {
        let decoded =
            base64::decode(s).map_err(|e| ClientError::Request(format!("Failed to decode cursor: {}", e).into()))?;
        let cursor_str = String::from_utf8(decoded)
            .map_err(|e| ClientError::Request(format!("Invalid cursor encoding: {}", e).into()))?;
        let cursor = serde_json::from_str(&cursor_str)
            .map_err(|e| ClientError::Request(format!("Invalid cursor format: {}", e).into()))?;
        Ok(cursor)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StdioConfig {
    pub command: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct SseConfig {
    #[serde(default = "default_server_url")]
    #[builder(default = default_server_url())]
    pub endpoint: String,
}

fn default_server_url() -> String {
    "http://127.0.0.1:8090".to_string()
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
    "ws://127.0.0.1:9090".to_string()
}

impl Default for WsConfig {
    fn default() -> Self {
        Self::builder().build()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "transport")]
pub enum TransportConfig {
    #[serde(rename = "stdio")]
    Stdio(StdioConfig),
    #[serde(rename = "sse")]
    Sse(SseConfig),
    #[serde(rename = "ws")]
    Ws(WsConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct ServerConfig {
    pub name: String,
    #[serde(flatten)]
    pub transport: TransportConfig,
    #[serde(default = "default_request_timeout")]
    #[builder(default = default_request_timeout())]
    pub request_timeout: u64,
}

fn default_request_timeout() -> u64 {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder, Default)]
pub struct ClientConfig {
    pub name: String,
    pub servers: Vec<ServerConfig>,
}

pub trait ModelContextProtocolClient: Send + Sync + 'static {
    fn get_server_configs(&self) -> impl Future<Output = Vec<ServerConfig>> + Send;
    fn get_capabilities(&self) -> impl Future<Output = ClientCapabilities> + Send;
    fn get_roots(&self) -> impl Future<Output = Vec<Root>> + Send;
    fn on_create_message(
        &self,
        params: CreateMessageRequestParams,
    ) -> impl Future<Output = Result<CreateMessageResult, ClientError>> + Send;
}

type ResponseSender = oneshot::Sender<Result<serde_json::Value, ClientError>>;
type ProgressSender = oneshot::Sender<ProgressNotificationParams>;
type PendingProgressRequests = Arc<Mutex<HashMap<ProgressToken, ProgressSender>>>;
type PendingClientRequests = Arc<Mutex<HashMap<RequestId, ResponseSender>>>;
type PendingServerRequests = Arc<Mutex<HashMap<RequestId, AbortHandle>>>;

#[derive(Clone)]
struct ServerConnection {
    transport: TransportType,
    transport_sender: TransportSender,
    server_capabilities: Arc<RwLock<Option<ServerCapabilities>>>,
    conn_id: ConnectionId,
    pending_requests: PendingClientRequests,
    pending_progress_requests: PendingProgressRequests,
    request_counter: Arc<RwLock<u64>>,
}

impl ServerConnection {
    async fn request<T: ModelContextProtocolClient, R: DeserializeOwned>(
        &mut self,
        method: String,
        params: serde_json::Value,
        progress: bool,
        client: Arc<RwLock<T>>,
    ) -> Result<Operation<R>, ClientError> {
        let mut counter = self.request_counter.write().await;
        *counter += 1;
        let id = *counter;

        let id = jsonrpc_core::Id::Num(id);
        let request_key = match MessageId::try_from(&id) {
            Ok(key) => key,
            Err(e) => return Err(ClientError::Request(format!("Invalid request ID: {}", e).into())),
        };

        let mut params_value = params;
        let mut progress_token = None;

        if progress {
            let token_value = uuid::Uuid::new_v4().to_string();
            progress_token = Some(token_value.clone());

            let meta = crate::schema::JsonrpcrequestParamsMeta { progress_token: Some(token_value.into()) };

            let rpc_params = crate::schema::JsonrpcrequestParams { meta: Some(meta) };

            let modified_params = crate::Params { params: params_value, rpc_params: Some(rpc_params) };

            params_value = serde_json::to_value(modified_params).map_err(|e| {
                ClientError::Request(format!("Failed to serialize params with progress token: {}", e).into())
            })?;
        }

        let request = jsonrpc_core::MethodCall {
            jsonrpc: Some(jsonrpc_core::Version::V2),
            method,
            params: Params::Map(params_value.as_object().cloned().unwrap_or_default()),
            id,
        };

        let (response_tx, response_rx) = oneshot::channel();
        let conn_id = self.conn_id.clone();
        let request_id = (conn_id.clone(), request_key.clone());

        {
            let mut pending = self.pending_requests.lock().await;
            pending.insert((conn_id.clone(), request_key.clone()), response_tx);
        }

        if let Err(e) = self.transport_sender.send(request.into(), conn_id.clone()).await {
            let mut pending = self.pending_requests.lock().await;
            pending.remove(&(conn_id, request_key));
            return Err(ClientError::Transport(format!("Send: {}", e).into()));
        }

        let timeout = client
            .read()
            .await
            .get_server_configs()
            .await
            .iter()
            .find(|cfg| self.conn_id.contains(&cfg.name))
            .map(|cfg| cfg.request_timeout)
            .unwrap_or(5);

        let pending_requests = self.pending_requests.clone();
        let request_id_clone = request_id.clone();
        let transport_sender = self.transport_sender.clone();

        let progress_rx = if progress {
            if let Some(token) = progress_token {
                let (progress_tx, progress_rx) = oneshot::channel();
                let mut pending_progress = self.pending_progress_requests.lock().await;
                pending_progress.insert(token.into(), progress_tx);
                Some(progress_rx)
            } else {
                None
            }
        } else {
            None
        };

        let future = async move {
            match tokio::time::timeout(std::time::Duration::from_secs(timeout), response_rx).await {
                Ok(response) => match response {
                    Ok(result) => match result {
                        Ok(json_value) => serde_json::from_value::<R>(json_value)
                            .map_err(|e| anyhow::anyhow!("Failed to deserialize response: {}", e)),
                        Err(e) => Err(anyhow::anyhow!("RPC error: {}", e)),
                    },
                    Err(e) => Err(anyhow::anyhow!("Response channel closed: {}", e)),
                },
                Err(_) => {
                    let mut pending = pending_requests.lock().await;
                    pending.remove(&request_id_clone);
                    Err(anyhow::anyhow!("Request timed out"))
                }
            }
        };

        Ok(Operation::new(request_id, future, transport_sender, progress_rx))
    }

    async fn notify(&mut self, method: String, params: serde_json::Value) -> Result<(), ClientError> {
        let notification = jsonrpc_core::Notification {
            jsonrpc: Some(jsonrpc_core::Version::V2),
            method,
            params: Params::Map(params.as_object().cloned().unwrap_or_default()),
        };

        let conn_id = self.conn_id.clone();

        self.transport_sender
            .send(notification.into(), conn_id)
            .await
            .map_err(|e| ClientError::Transport(format!("Send: {}", e).into()))
    }
}

#[derive(Clone)]
pub struct Client<T: ModelContextProtocolClient> {
    client: Arc<RwLock<T>>,
    connections: HashMap<String, ServerConnection>,
    io_handler: MetaIoHandler<ClientMetadata>,
    roots: Arc<RwLock<HashMap<String, Root>>>,
    pending_server_requests: PendingServerRequests,
}

impl<T: ModelContextProtocolClient> Client<T> {
    pub async fn new(client: T) -> Result<Self, ClientError> {
        let client = Arc::new(RwLock::new(client));
        let server_configs = client.read().await.get_server_configs().await;

        if server_configs.is_empty() {
            return Err(ClientError::Request("No server configurations available".into()));
        }

        let mut io_handler = MetaIoHandler::default();

        io_handler.add_method_with_meta("sampling/createMessage", {
            let client = client.clone();
            move |params: Params, _meta: ClientMetadata| {
                let client = client.clone();
                async move {
                    let params: CreateMessageRequestParams = match params.parse() {
                        Ok(params) => params,
                        Err(e) => {
                            error!("Failed to parse createMessage parameters: {}", e);
                            return Err(jsonrpc_core::Error::invalid_params(e.to_string()));
                        }
                    };
                    let result = client.read().await.on_create_message(params).await;

                    match result {
                        Ok(result) => {
                            info!("Successfully handled createMessage request");
                            Ok(serde_json::to_value(result).map_err(|e| {
                                error!("Failed to serialize createMessage result: {}", e);
                                jsonrpc_core::Error::invalid_params(e.to_string())
                            })?)
                        }
                        Err(e) => {
                            error!("Failed to handle createMessage request: {}", e);
                            Err(jsonrpc_core::Error::invalid_params(e.to_string()))
                        }
                    }
                }
            }
        });

        io_handler.add_method_with_meta("roots/list", {
            let client = client.clone();
            move |_params: Params, _meta: ClientMetadata| {
                let client = client.clone();
                async move {
                    let roots = client.read().await.get_roots().await;
                    info!("Successfully handled roots/list request");
                    Ok(serde_json::to_value(roots).map_err(|e| {
                        error!("Failed to serialize roots/list result: {}", e);
                        jsonrpc_core::Error::invalid_params(e.to_string())
                    })?)
                }
            }
        });

        io_handler.add_notification_with_meta("notifications/message", move |params: Params, _meta: ClientMetadata| {
            tokio::spawn(async move {
                match params.parse::<LoggingMessageNotificationParams>() {
                    Ok(params) => {
                        debug!("[{}] Server Log: {:?}", params.logger.clone().unwrap_or_default(), params);
                    }
                    Err(e) => {
                        error!("Failed to parse notifications/message parameters: {}", e);
                    }
                }
            });
        });

        io_handler.add_notification_with_meta("notifications/progress", move |params: Params, meta: ClientMetadata| {
            tokio::spawn(async move {
                match params.parse::<ProgressNotificationParams>() {
                    Ok(params) => {
                        let total_info =
                            params.total.map_or_else(|| String::from("unknown"), |total| format!("{}", total));
                        info!(
                            "[{}] Progress: {}/{} - {}",
                            meta.conn_id,
                            params.progress,
                            total_info,
                            params.message.unwrap_or_else(|| String::from(""))
                        );
                    }
                    Err(e) => {
                        error!("Failed to parse notifications/progress parameters: {}", e);
                    }
                }
            });
        });

        let pending_server_requests: Arc<Mutex<HashMap<(ConnectionId, MessageId), AbortHandle>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let pending_server_requests_clone = pending_server_requests.clone();

        io_handler.add_notification_with_meta(
            "notifications/cancelled",
            move |params: Params, meta: ClientMetadata| {
                let pending_server_requests = pending_server_requests_clone.clone();

                tokio::spawn(async move {
                    match params.parse::<CancelledNotificationParams>() {
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

                            match MessageId::try_from(&id) {
                                Ok(request_key) => {
                                    info!(
                                        "Server requested cancellation of request {}: {}",
                                        request_key,
                                        cancel_params.reason.as_deref().unwrap_or("No reason provided")
                                    );

                                    let mut active_reqs_lock = pending_server_requests.lock().await;
                                    let conn_id = meta.conn_id.clone();
                                    let key = (conn_id.clone(), request_key.clone());

                                    if let Some(abort_handle) = active_reqs_lock.remove(&key) {
                                        abort_handle.abort();
                                        info!(
                                            "Successfully aborted processing for server request {} on connection {}",
                                            request_key, conn_id
                                        );
                                    } else {
                                        debug!(
                                            "Server request {} not found or already completed for connection {}",
                                            request_key, conn_id
                                        );
                                    }
                                }
                                Err(e) => {
                                    error!("Received cancellation with invalid request ID type: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to parse cancellation notification parameters: {}", e);
                        }
                    }
                });
            },
        );

        let mut client = Self {
            client,
            connections: HashMap::new(),
            io_handler,
            roots: Arc::new(RwLock::new(HashMap::new())),
            pending_server_requests,
        };

        for config in server_configs {
            let name = config.name.clone();
            client.add_server(name, config).await?;
        }

        Ok(client)
    }

    pub async fn add_server(&mut self, name: String, server_config: ServerConfig) -> Result<(), ClientError> {
        let (on_message_tx, mut on_message_rx) = mpsc::channel::<JsonRpcMessage>(1);
        let (on_error_tx, _on_error_rx) = mpsc::channel::<Error>(1);
        let (on_close_tx, _on_close_rx) = mpsc::channel::<()>(1);

        let mut transport = match &server_config.transport {
            TransportConfig::Stdio(config) => {
                let transport = StdioTransport::new_client(&config, on_message_tx, on_error_tx, on_close_tx).await;
                let transport = match transport {
                    Ok(transport) => transport,
                    Err(e) => return Err(ClientError::Transport(format!("Client new: {}", e).into())),
                };
                TransportType::Stdio(transport)
            }
            TransportConfig::Sse(config) => {
                let transport = SseTransport::new_client(config, on_message_tx, on_error_tx, on_close_tx);
                let transport = match transport {
                    Ok(transport) => transport,
                    Err(e) => return Err(ClientError::Transport(format!("Client new: {}", e).into())),
                };
                TransportType::Sse(transport)
            }
            TransportConfig::Ws(config) => {
                let transport = WsTransport::new_client(config, on_message_tx, on_error_tx, on_close_tx);
                let transport = match transport {
                    Ok(transport) => transport,
                    Err(e) => return Err(ClientError::Transport(format!("Client new: {}", e).into())),
                };
                TransportType::Ws(transport)
            }
        };

        let conn_id = ConnectionId::new(Some(server_config.name.clone()));
        let transport_sender = transport.sender();
        let conn_id_clone = conn_id.clone();

        let transport_sender_clone = transport_sender.clone();
        let io_handler_clone = self.io_handler.clone();
        let _start_handle =
            transport.start().await.map_err(|e| ClientError::Transport(format!("Start: {}", e).into()))?;

        let pending_requests = Arc::new(Mutex::new(HashMap::<(ConnectionId, MessageId), ResponseSender>::new()));
        let pending_requests_clone = pending_requests.clone();
        let pending_server_requests_clone = self.pending_server_requests.clone();

        let _message_handler = tokio::spawn({
            let pending_requests = pending_requests_clone;
            let io_handler_clone = io_handler_clone;
            let transport_sender_clone = transport_sender_clone;
            let conn_id_clone = conn_id_clone.clone();
            let pending_server_requests = pending_server_requests_clone;

            async move {
                while let Some(message) = on_message_rx.recv().await {
                    match &message {
                        JsonRpcMessage::Response(response) => match response {
                            jsonrpc_core::Response::Single(output) => match output {
                                jsonrpc_core::Output::Success(success) => {
                                    if let jsonrpc_core::Id::Num(_) = success.id {
                                        let mut requests = pending_requests.lock().await;
                                        if let Ok(key) = MessageId::try_from(&success.id) {
                                            let conn_id_for_key = conn_id_clone.clone();
                                            if let Some(sender) = requests.remove(&(conn_id_for_key, key)) {
                                                let _ = sender.send(Ok(success.result.clone()));
                                            }
                                        }
                                    }
                                }
                                jsonrpc_core::Output::Failure(failure) => {
                                    if let jsonrpc_core::Id::Num(_) = failure.id {
                                        let mut requests = pending_requests.lock().await;
                                        if let Ok(key) = MessageId::try_from(&failure.id) {
                                            let conn_id_for_key = conn_id_clone.clone();
                                            if let Some(sender) = requests.remove(&(conn_id_for_key, key)) {
                                                let _ = sender.send(Err(ClientError::Request(
                                                    format!("RPC error: {:?}", failure.error).into(),
                                                )));
                                            }
                                        }
                                    }
                                }
                            },
                            jsonrpc_core::Response::Batch(_) => {
                                warn!("Unsupported batch response");
                            }
                        },
                        JsonRpcMessage::Request(request) => match request {
                            jsonrpc_core::Request::Single(jsonrpc_core::Call::MethodCall(call)) => {
                                match MessageId::try_from(&call.id) {
                                    Ok(request_key) => {
                                        let request_clone = request.clone();
                                        let io_handler_clone_inner = io_handler_clone.clone();
                                        let transport_sender_clone_inner = transport_sender_clone.clone();
                                        let active_reqs = pending_server_requests.clone();

                                        let request_key_clone = request_key.clone();
                                        let conn_id_for_closure = conn_id_clone.clone();
                                        let conn_id_for_key = conn_id_clone.clone();

                                        let abort_handle = {
                                            let handle = tokio::spawn(async move {
                                                if let Some(response) = io_handler_clone_inner
                                                    .handle_rpc_request(
                                                        request_clone,
                                                        ClientMetadata { conn_id: conn_id_for_closure.clone() },
                                                    )
                                                    .await
                                                {
                                                    if let Err(e) = transport_sender_clone_inner
                                                        .send(response.into(), conn_id_for_closure.clone())
                                                        .await
                                                    {
                                                        error!("Failed to send response: {}", e);
                                                    }
                                                }

                                                let mut active_reqs_lock = active_reqs.lock().await;
                                                active_reqs_lock.remove(&(conn_id_for_key, request_key_clone));
                                            });

                                            handle.abort_handle()
                                        };

                                        let mut active_reqs = pending_server_requests.lock().await;
                                        let conn_id_for_insert = conn_id_clone.clone();
                                        active_reqs.insert((conn_id_for_insert, request_key), abort_handle);
                                    }
                                    Err(err) => {
                                        warn!("Received method call with unsupported ID type: {}", err);
                                    }
                                }
                            }
                            jsonrpc_core::Request::Single(jsonrpc_core::Call::Notification(notification)) => {
                                let conn_id_for_notification = conn_id_clone.clone();
                                if let Some(result) = io_handler_clone
                                    .handle_rpc_request(
                                        jsonrpc_core::Request::Single(jsonrpc_core::Call::Notification(
                                            notification.clone(),
                                        )),
                                        ClientMetadata { conn_id: conn_id_for_notification },
                                    )
                                    .await
                                {
                                    debug!("Notification handled successfully {:?}", result);
                                }
                            }
                            _ => {
                                warn!("Unsupported batch request: {:?}", request);
                            }
                        },
                    }
                }
            }
        });

        let server_connection = ServerConnection {
            transport,
            transport_sender,
            server_capabilities: Arc::new(RwLock::new(None)),
            request_counter: Arc::new(RwLock::new(0)),
            pending_requests,
            pending_progress_requests: PendingProgressRequests::default(),
            conn_id,
        };

        self.connections.insert(name, server_connection);
        Ok(())
    }

    fn list_servers(&self) -> Vec<String> {
        self.connections.keys().cloned().collect()
    }

    pub async fn initialize(
        &mut self,
        client_info: Implementation,
    ) -> Result<HashMap<String, InitializeResult>, ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let mut results = HashMap::new();
        let mut errors = Vec::new();
        let client = self.client.clone();

        for (server_name, connection) in &mut self.connections {
            let capabilities = client.read().await.get_capabilities().await;
            let params = InitializeRequestParams {
                protocol_version: "2024-11-05".to_string(),
                capabilities,
                client_info: client_info.clone(),
            };

            match connection
                .request::<_, InitializeResult>(
                    "initialize".to_string(),
                    serde_json::to_value(params)?,
                    false,
                    client.clone(),
                )
                .await
            {
                Ok(operation) => match operation.await {
                    Ok(result) => {
                        {
                            let mut server_capabilities = connection.server_capabilities.write().await;
                            *server_capabilities = Some(result.capabilities.clone());
                        }
                        results.insert(server_name.clone(), result);
                    }
                    Err(e) => {
                        errors.push(format!("Failed to deserialize initialize result from '{}': {:?}", server_name, e));
                    }
                },
                Err(e) => {
                    errors.push(format!("Failed to create initialize request for '{}': {:?}", server_name, e));
                }
            }
        }

        if !errors.is_empty() {
            warn!("Some servers failed to initialize: {}", errors.join(", "));
        }

        if results.is_empty() {
            Err(ClientError::Request("All servers failed to initialize".into()))
        } else {
            Ok(results)
        }
    }

    pub async fn initialized(&mut self) -> Result<(), ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let mut errors = Vec::new();

        for (server_name, connection) in &mut self.connections {
            let params = InitializedNotificationParams { meta: None };
            if let Err(e) =
                connection.notify("notifications/initialized".to_string(), serde_json::to_value(params)?).await
            {
                errors.push(format!("Failed to send initialized notification to '{}': {:?}", server_name, e));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ClientError::Request(errors.join(", ").into()))
        }
    }

    pub async fn list_resources(
        &mut self,
        params: Option<ListResourcesRequestParams>,
    ) -> Result<Operation<ListResourcesResult>, ClientError> {
        self.list_items::<ListResourcesResult, ListResourcesRequestParams>("resources/list", params).await
    }

    pub async fn list_all_resources(
        &mut self,
        params: Option<ListResourcesRequestParams>,
    ) -> Result<Operation<Vec<Resource>>, ClientError> {
        self.list_all_items::<ListResourcesResult, ListResourcesRequestParams>("resources/list", params).await
    }

    pub async fn read_resource(
        &mut self,
        params: ReadResourceRequestParams,
    ) -> Result<Operation<ReadResourceResult>, ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let connections = self.connections.clone();
        let client = self.client.clone();
        let params_clone = params.clone();

        let future = async move {
            let mut errors = Vec::new();

            for (server_name, mut connection) in connections {
                match connection
                    .request::<_, ReadResourceResult>(
                        "resources/read".to_string(),
                        serde_json::to_value(params_clone.clone())?,
                        client.clone(),
                    )
                    .await
                {
                    Ok(operation) => match operation.await {
                        Ok(result) => return Ok(result),
                        Err(e) => {
                            errors.push(format!("Error from '{}': {:?}", server_name, e));
                        }
                    },
                    Err(e) => {
                        errors.push(format!("Error from '{}': {:?}", server_name, e));
                    }
                }
            }

            Err(anyhow::anyhow!("Unable to read resource from any server: {}", errors.join(", ")))
        };

        Ok(Operation::new_multiple(future))
    }

    pub async fn list_resource_templates(
        &mut self,
        params: Option<ListResourceTemplatesRequestParams>,
    ) -> Result<Operation<ListResourceTemplatesResult>, ClientError> {
        self.list_items::<ListResourceTemplatesResult, ListResourceTemplatesRequestParams>(
            "resources/templates/list",
            params,
        )
        .await
    }

    pub async fn list_all_resource_templates(
        &mut self,
        params: Option<ListResourceTemplatesRequestParams>,
    ) -> Result<Operation<Vec<ResourceTemplate>>, ClientError> {
        self.list_all_items::<ListResourceTemplatesResult, ListResourceTemplatesRequestParams>(
            "resources/templates/list",
            params,
        )
        .await
    }

    pub async fn subscribe_resource(&mut self, uri: String) -> Result<Operation<()>, ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let connections = self.connections.clone();
        let client = self.client.clone();
        let uri_clone = uri.clone();
        let params = serde_json::json!({ "uri": uri });

        let future = async move {
            let mut errors = Vec::new();

            for (server_name, mut connection) in connections {
                match connection
                    .request::<_, serde_json::Value>("resources/subscribe".to_string(), params.clone(), client.clone())
                    .await
                {
                    Ok(operation) => match operation.await {
                        Ok(_) => return Ok(()),
                        Err(e) => {
                            errors.push(format!("Error from '{}': {:?}", server_name, e));
                        }
                    },
                    Err(e) => {
                        errors.push(format!("Error from '{}': {:?}", server_name, e));
                    }
                }
            }

            Err(anyhow::anyhow!("Unable to subscribe resource {} on any server: {}", uri_clone, errors.join(", ")))
        };

        Ok(Operation::new_multiple(future))
    }

    pub async fn unsubscribe_resource(&mut self, uri: String) -> Result<Operation<()>, ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let connections = self.connections.clone();
        let client = self.client.clone();
        let uri_clone = uri.clone();
        let params = serde_json::json!({ "uri": uri });

        let future = async move {
            let mut errors = Vec::new();

            for (server_name, mut connection) in connections {
                match connection
                    .request::<_, serde_json::Value>(
                        "resources/unsubscribe".to_string(),
                        params.clone(),
                        client.clone(),
                    )
                    .await
                {
                    Ok(operation) => match operation.await {
                        Ok(_) => return Ok(()),
                        Err(e) => {
                            errors.push(format!("Error from '{}': {:?}", server_name, e));
                        }
                    },
                    Err(e) => {
                        errors.push(format!("Error from '{}': {:?}", server_name, e));
                    }
                }
            }

            Err(anyhow::anyhow!("Unable to unsubscribe resource {} on any server: {}", uri_clone, errors.join(", ")))
        };

        Ok(Operation::new_multiple(future))
    }

    pub async fn list_prompts(
        &mut self,
        params: Option<ListPromptsRequestParams>,
    ) -> Result<Operation<ListPromptsResult>, ClientError> {
        self.list_items::<ListPromptsResult, ListPromptsRequestParams>("prompts/list", params).await
    }

    pub async fn list_all_prompts(
        &mut self,
        params: Option<ListPromptsRequestParams>,
    ) -> Result<Operation<Vec<Prompt>>, ClientError> {
        self.list_all_items::<ListPromptsResult, ListPromptsRequestParams>("prompts/list", params).await
    }

    pub async fn get_prompt(
        &mut self,
        params: GetPromptRequestParams,
    ) -> Result<Operation<GetPromptResult>, ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let connections = self.connections.clone();
        let client = self.client.clone();
        let params_clone = params.clone();

        let future = async move {
            let mut errors = Vec::new();

            for (server_name, mut connection) in connections {
                match connection
                    .request::<_, GetPromptResult>(
                        "prompts/get".to_string(),
                        serde_json::to_value(params_clone.clone())?,
                        client.clone(),
                    )
                    .await
                {
                    Ok(operation) => match operation.await {
                        Ok(result) => return Ok(result),
                        Err(e) => {
                            errors.push(format!("Error from '{}': {:?}", server_name, e));
                        }
                    },
                    Err(e) => {
                        errors.push(format!("Error from '{}': {:?}", server_name, e));
                    }
                }
            }

            Err(anyhow::anyhow!("Unable to get prompt from any server: {}", errors.join(", ")))
        };

        Ok(Operation::new_multiple(future))
    }

    pub async fn list_tools(
        &mut self,
        params: Option<ListToolsRequestParams>,
    ) -> Result<Operation<ListToolsResult>, ClientError> {
        self.list_items::<ListToolsResult, ListToolsRequestParams>("tools/list", params).await
    }

    pub async fn list_all_tools(
        &mut self,
        params: Option<ListToolsRequestParams>,
    ) -> Result<Operation<Vec<Tool>>, ClientError> {
        self.list_all_items::<ListToolsResult, ListToolsRequestParams>("tools/list", params).await
    }

    pub async fn call_tool(&mut self, params: CallToolRequestParams) -> Result<Operation<CallToolResult>, ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let connections = self.connections.clone();
        let client = self.client.clone();
        let params_clone = params.clone();

        let future = async move {
            let mut errors = Vec::new();

            for (server_name, mut connection) in connections {
                match connection
                    .request::<_, CallToolResult>(
                        "tools/call".to_string(),
                        serde_json::to_value(params_clone.clone())?,
                        client.clone(),
                    )
                    .await
                {
                    Ok(operation) => match operation.await {
                        Ok(result) => return Ok(result),
                        Err(e) => {
                            errors.push(format!("Error from '{}': {:?}", server_name, e));
                        }
                    },
                    Err(e) => {
                        errors.push(format!("Error from '{}': {:?}", server_name, e));
                    }
                }
            }

            Err(anyhow::anyhow!("Unable to call tool on any server: {}", errors.join(", ")))
        };

        Ok(Operation::new_multiple(future))
    }

    async fn complete(&mut self, params: CompleteRequestParams) -> Result<Operation<CompleteResult>, ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let connections = self.connections.clone();
        let client = self.client.clone();
        let params_clone = params.clone();

        let future = async move {
            let mut errors = Vec::new();
            let mut no_support_count = 0;

            for (server_name, mut connection) in connections {
                let supports_completions = {
                    let server_caps = connection.server_capabilities.read().await;
                    server_caps.as_ref().and_then(|caps| caps.completions.as_ref()).is_some()
                };

                if !supports_completions {
                    no_support_count += 1;
                    continue;
                }

                match connection
                    .request::<_, CompleteResult>(
                        "completion/complete".to_string(),
                        serde_json::to_value(params_clone.clone())?,
                        client.clone(),
                    )
                    .await
                {
                    Ok(operation) => match operation.await {
                        Ok(result) => return Ok(result),
                        Err(e) => {
                            errors.push(format!("Error from '{}': {:?}", server_name, e));
                        }
                    },
                    Err(e) => {
                        errors.push(format!("Error from '{}': {:?}", server_name, e));
                    }
                }
            }

            if errors.is_empty() && no_support_count > 0 {
                return Ok(CompleteResult {
                    meta: None,
                    completion: CompleteResultCompletion { values: vec![], total: None, has_more: None },
                });
            }

            Err(anyhow::anyhow!("Unable to complete on any server: {}", errors.join(", ")))
        };

        Ok(Operation::new_multiple(future))
    }

    pub async fn complete_prompt(
        &mut self,
        prompt_name: String,
        name: String,
        value: String,
    ) -> Result<Operation<CompleteResult>, ClientError> {
        let prompt_ref = PromptReference { type_: "ref/prompt".to_string(), name: prompt_name };

        let params = CompleteRequestParams {
            ref_: serde_json::to_value(prompt_ref)?,
            argument: CompleteRequestParamsArgument { name, value },
        };

        self.complete(params).await
    }

    pub async fn complete_resource(
        &mut self,
        resource_uri: String,
        name: String,
        value: String,
    ) -> Result<Operation<CompleteResult>, ClientError> {
        let resource_ref = ResourceReference { type_: "ref/resource".to_string(), uri: resource_uri };

        let params = CompleteRequestParams {
            ref_: serde_json::to_value(resource_ref)?,
            argument: CompleteRequestParamsArgument { name, value },
        };

        self.complete(params).await
    }

    pub async fn add_root(&mut self, root: Root, meta: Option<BTreeMap<String, Value>>) -> Result<(), ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let client_capabilities = self.client.read().await.get_capabilities().await;
        let supports_root_notifications =
            client_capabilities.roots.map_or(false, |roots| roots.list_changed.unwrap_or(false));

        let root_is_new = {
            let mut client_roots = self.roots.write().await;
            client_roots.insert(root.uri.clone(), root.clone()).is_none()
        };

        if !root_is_new || !supports_root_notifications {
            return Ok(());
        }

        let mut errors = Vec::new();

        for (server_name, connection) in &mut self.connections {
            let params = RootsListChangedNotificationParams { meta: meta.clone() };
            if let Err(e) =
                connection.notify("notifications/roots/list_changed".to_string(), serde_json::to_value(params)?).await
            {
                errors.push(format!("Failed to notify root change on '{}': {:?}", server_name, e));
            }
        }

        if !errors.is_empty() {
            warn!("Failed to add root to some servers: {}", errors.join(", "));
        }

        Ok(())
    }

    pub async fn remove_root(&mut self, uri: String, meta: Option<BTreeMap<String, Value>>) -> Result<(), ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let client_capabilities = self.client.read().await.get_capabilities().await;
        let supports_root_notifications =
            client_capabilities.roots.map_or(false, |roots| roots.list_changed.unwrap_or(false));

        let root_existed = {
            let mut client_roots = self.roots.write().await;
            client_roots.remove(&uri).is_some()
        };

        if !root_existed || !supports_root_notifications {
            return Ok(());
        }

        let mut errors = Vec::new();

        for (server_name, connection) in &mut self.connections {
            let params = RootsListChangedNotificationParams { meta: meta.clone() };
            if let Err(e) =
                connection.notify("notifications/roots/list_changed".to_string(), serde_json::to_value(params)?).await
            {
                errors.push(format!("Failed to notify root removal on '{}': {:?}", server_name, e));
            }
        }

        if !errors.is_empty() {
            warn!("Failed to remove root from some servers: {}", errors.join(", "));
        }

        Ok(())
    }

    pub async fn close(&mut self) -> Result<(), ClientError> {
        if self.connections.is_empty() {
            return Ok(());
        }

        let mut errors = Vec::new();

        for (server_name, connection) in &mut self.connections {
            if let Err(e) = connection.transport.close().await {
                errors.push(format!("Failed to close '{}': {}", server_name, e));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ClientError::Request(errors.join(", ").into()))
        }
    }

    async fn list_items<R, P>(&mut self, endpoint: &str, params: Option<P>) -> Result<Operation<R>, ClientError>
    where
        R: ListResult + DeserializeOwned + Send + 'static,
        P: ListRequestParams + Send + 'static,
    {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let initial_server_cursors: HashMap<String, Option<String>> = if let Some(params) = &params {
            if let Some(cursor) = params.get_cursor() {
                match MultiServerCursor::from_string(cursor) {
                    Ok(multi_cursor) => multi_cursor.server_cursors,
                    Err(e) => return Err(e),
                }
            } else {
                self.connections.keys().map(|k| (k.clone(), None)).collect()
            }
        } else {
            self.connections.keys().map(|k| (k.clone(), None)).collect()
        };

        let client_clone = self.client.clone();
        let connections = self.connections.clone();
        let endpoint = endpoint.to_string();
        let params = params.clone();

        let future = async move {
            let mut all_items = Vec::new();
            let mut errors = Vec::new();
            let mut has_more = false;
            let mut server_cursors = HashMap::new();

            for (server_name, cursor) in initial_server_cursors {
                if let Some(mut connection) = connections.get(&server_name).cloned() {
                    let req_params = match params.clone() {
                        Some(p) => p.with_cursor(cursor),
                        None => P::default().with_cursor(cursor),
                    };

                    match connection
                        .request::<_, R>(
                            endpoint.clone(),
                            serde_json::to_value(req_params)?,
                            false,
                            client_clone.clone(),
                        )
                        .await
                    {
                        Ok(operation) => match operation.await {
                            Ok(result) => {
                                all_items.extend(result.get_items().to_vec());

                                if let Some(next_cursor) = result.get_next_cursor() {
                                    server_cursors.insert(server_name, Some(next_cursor.to_string()));
                                    has_more = true;
                                }
                            }
                            Err(e) => {
                                errors.push(format!("Failed to list items from '{}': {:?}", server_name, e));
                            }
                        },
                        Err(e) => {
                            errors.push(format!("Error from '{}': {:?}", server_name, e));
                        }
                    }
                }
            }

            if all_items.is_empty() && !errors.is_empty() {
                return Err(anyhow::anyhow!("All servers failed: {}", errors.join(", ")));
            }

            let next_cursor = if has_more {
                let multi_cursor = MultiServerCursor { server_cursors };
                match multi_cursor.to_string() {
                    Ok(cursor) => Some(cursor),
                    Err(e) => {
                        tracing::warn!("Failed to encode multi-server cursor: {:?}", e);
                        None
                    }
                }
            } else {
                None
            };

            Ok(R::with_items_and_cursor(all_items, next_cursor, None))
        };

        Ok(Operation::new_multiple(future))
    }

    async fn list_all_items<R, P>(
        &mut self,
        endpoint: &str,
        params: Option<P>,
    ) -> Result<Operation<Vec<R::Item>>, ClientError>
    where
        R: ListResult + DeserializeOwned + Send + 'static,
        P: ListRequestParams + Send + 'static,
    {
        let endpoint = endpoint.to_string();
        let client = self.client.clone();
        let connections = self.connections.clone();
        let params = params.clone();

        let future = async move {
            let mut all_items = Vec::new();
            let mut next_cursor = None;
            let mut client_instance = Client {
                client,
                connections,
                io_handler: MetaIoHandler::default(),
                roots: Arc::new(RwLock::new(HashMap::new())),
                pending_server_requests: Arc::new(Mutex::new(HashMap::new())),
            };

            loop {
                let params_with_cursor = match params.clone() {
                    Some(p) => Some(p.with_cursor(next_cursor)),
                    None => Some(P::default().with_cursor(next_cursor)),
                };

                let operation = client_instance.list_items::<R, P>(&endpoint, params_with_cursor).await?;
                let result = operation.await?;

                all_items.extend(result.get_items().to_vec());

                next_cursor = result.get_next_cursor().map(|s| s.to_string());
                if next_cursor.is_none() {
                    break;
                }
            }

            Ok(all_items)
        };

        Ok(Operation::new_multiple(future))
    }

    pub fn iter_resources<'a>(
        &'a mut self,
        params: Option<ListResourcesRequestParams>,
    ) -> ItemsIterator<'a, T, ListResourcesResult, ListResourcesRequestParams> {
        ItemsIterator::new(self, "resources/list".to_string(), params)
    }

    pub fn iter_prompts<'a>(
        &'a mut self,
        params: Option<ListPromptsRequestParams>,
    ) -> ItemsIterator<'a, T, ListPromptsResult, ListPromptsRequestParams> {
        ItemsIterator::new(self, "prompts/list".to_string(), params)
    }

    pub fn iter_tools<'a>(
        &'a mut self,
        params: Option<ListToolsRequestParams>,
    ) -> ItemsIterator<'a, T, ListToolsResult, ListToolsRequestParams> {
        ItemsIterator::new(self, "tools/list".to_string(), params)
    }

    pub fn iter_resource_templates<'a>(
        &'a mut self,
        params: Option<ListResourceTemplatesRequestParams>,
    ) -> ItemsIterator<'a, T, ListResourceTemplatesResult, ListResourceTemplatesRequestParams> {
        ItemsIterator::new(self, "resources/templates/list".to_string(), params)
    }

    pub async fn set_log_level(&mut self, level: LoggingLevel) -> Result<Operation<(usize, Vec<String>)>, ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let connections = self.connections.clone();
        let client = self.client.clone();
        let level_clone = level.clone();
        let server_names = self.connections.keys().cloned().collect::<Vec<_>>();

        let future = async move {
            let mut success_count = 0;
            let mut errors = Vec::new();

            for server_name in server_names {
                if let Some(mut connection) = connections.get(&server_name).cloned() {
                    let supports_logging = {
                        let server_caps = connection.server_capabilities.read().await;
                        server_caps.as_ref().and_then(|caps| caps.logging.as_ref()).is_some()
                    };

                    if !supports_logging {
                        errors.push(format!("Server '{}' does not support logging", server_name));
                        continue;
                    }

                    let params = crate::schema::SetLevelRequestParams { level: level_clone.clone() };

                    match connection
                        .request::<_, serde_json::Value>(
                            "logging/setLevel".to_string(),
                            serde_json::to_value(params)?,
                            client.clone(),
                        )
                        .await
                    {
                        Ok(operation) => match operation.await {
                            Ok(_) => {
                                info!("Set log level for server '{}' to {:?}", server_name, level_clone);
                                success_count += 1;
                            }
                            Err(e) => {
                                errors.push(format!("Failed to set log level on '{}': {:?}", server_name, e));
                            }
                        },
                        Err(e) => errors.push(format!("Failed to set log level for '{}': {}", server_name, e)),
                    }
                }
            }

            if success_count > 0 {
                Ok((success_count, errors))
            } else {
                Err(anyhow::anyhow!("Failed to set log level on any server: {}", errors.join(", ")))
            }
        };

        Ok(Operation::new_multiple(future))
    }

    pub async fn cancel(&mut self, request_id: RequestId, reason: Option<String>) -> Result<(), ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let (connection_id, message_id) = request_id;
        let connection = self
            .connections
            .get_mut(&connection_id.to_string())
            .ok_or_else(|| ClientError::Request(format!("Server connection '{}' not found", connection_id).into()))?;

        let conn_id = connection.conn_id.clone();
        let request_exists = {
            let pending = connection.pending_requests.lock().await;
            pending.contains_key(&(conn_id.clone(), message_id.clone()))
        };

        if !request_exists {
            return Err(ClientError::Request(
                format!("Request {:?} not found or already completed", message_id).into(),
            ));
        }

        if Self::is_initialize_request(&message_id) {
            return Err(ClientError::Request(format!("Cannot cancel initialize request (ID: {})", message_id).into()));
        }

        let id_value = match &message_id {
            MessageId::Num(n) => serde_json::Value::Number(serde_json::Number::from(*n)),
            MessageId::Str(s) => serde_json::Value::String(s.clone()),
        };

        let params = CancelledNotificationParams { request_id: id_value, reason };
        let message_id_clone = message_id.clone();

        match connection.notify("notifications/cancelled".to_string(), serde_json::to_value(params)?).await {
            Ok(_) => {
                let mut pending = connection.pending_requests.lock().await;
                pending.remove(&(conn_id.clone(), message_id_clone));

                info!("Cancelled request {} on server '{}'", message_id, connection_id);
                Ok(())
            }
            Err(e) => {
                Err(ClientError::Request(format!("Failed to send cancellation to '{}': {:?}", connection_id, e).into()))
            }
        }
    }

    fn is_initialize_request(message_id: &MessageId) -> bool {
        match message_id {
            MessageId::Num(n) => *n == 1,
            _ => false,
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ClientError {
    #[error("Invalid transport type: {0}")]
    Transport(Cow<'static, str>),
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("Request: {0}")]
    Request(Cow<'static, str>),
    #[error("Client rejected sampling request")]
    SamplingRequestRejected,
}

impl<T: ModelContextProtocolClient> std::fmt::Debug for Client<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").field("servers", &self.list_servers()).finish()
    }
}

pub struct ItemsIterator<'a, T, R, P>
where
    T: ModelContextProtocolClient,
    R: ListResult + DeserializeOwned + Send + 'static,
    P: ListRequestParams + Send + 'static,
{
    client: &'a mut Client<T>,
    endpoint: String,
    params: Option<P>,
    next_cursor: Option<String>,
    done: bool,
    _phantom: std::marker::PhantomData<R>,
}

impl<'a, T, R, P> ItemsIterator<'a, T, R, P>
where
    T: ModelContextProtocolClient,
    R: ListResult + DeserializeOwned + Send + 'static,
    P: ListRequestParams + Send + 'static,
{
    pub fn new(client: &'a mut Client<T>, endpoint: String, params: Option<P>) -> Self {
        Self { client, endpoint, params, next_cursor: None, done: false, _phantom: std::marker::PhantomData }
    }

    pub async fn next(&mut self) -> Option<Result<Vec<R::Item>, ClientError>> {
        if self.done {
            return None;
        }

        let params_with_cursor = match self.params.clone() {
            Some(p) => Some(p.with_cursor(self.next_cursor.clone())),
            None => Some(P::default().with_cursor(self.next_cursor.clone())),
        };

        match self.client.list_items::<R, P>(&self.endpoint, params_with_cursor).await {
            Ok(operation) => match operation.await {
                Ok(result) => {
                    let items = result.get_items().to_vec();
                    if items.is_empty() {
                        self.done = true;
                        return None;
                    }

                    self.next_cursor = result.get_next_cursor().map(|s| s.to_string());
                    self.done = self.next_cursor.is_none();
                    Some(Ok(items))
                }
                Err(e) => {
                    self.done = true;
                    Some(Err(ClientError::Request(format!("Operation failed: {}", e).into())))
                }
            },
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

pub trait ListResult: serde::de::DeserializeOwned + Serialize {
    type Item: Clone + Send + 'static;

    fn get_items(&self) -> &[Self::Item];
    fn get_next_cursor(&self) -> Option<&str>;
    fn with_items_and_cursor(
        items: Vec<Self::Item>,
        next_cursor: Option<String>,
        meta: Option<BTreeMap<String, Value>>,
    ) -> Self;
}

pub trait ListRequestParams: serde::de::DeserializeOwned + Serialize + Clone + Default {
    fn with_cursor(&self, cursor: Option<String>) -> Self;
    fn get_cursor(&self) -> Option<&str>;
}

impl ListResult for ListResourcesResult {
    type Item = Resource;

    fn get_items(&self) -> &[Self::Item] {
        &self.resources
    }

    fn get_next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_deref()
    }

    fn with_items_and_cursor(
        items: Vec<Self::Item>,
        next_cursor: Option<String>,
        meta: Option<BTreeMap<String, Value>>,
    ) -> Self {
        Self { resources: items, next_cursor, meta }
    }
}

impl ListRequestParams for ListResourcesRequestParams {
    fn with_cursor(&self, cursor: Option<String>) -> Self {
        let mut params = self.clone();
        params.cursor = cursor;
        params
    }

    fn get_cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }
}
impl ListResult for ListPromptsResult {
    type Item = Prompt;

    fn get_items(&self) -> &[Self::Item] {
        &self.prompts
    }

    fn get_next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_deref()
    }

    fn with_items_and_cursor(
        items: Vec<Self::Item>,
        next_cursor: Option<String>,
        meta: Option<BTreeMap<String, Value>>,
    ) -> Self {
        Self { prompts: items, next_cursor, meta }
    }
}

impl ListRequestParams for ListPromptsRequestParams {
    fn with_cursor(&self, cursor: Option<String>) -> Self {
        let mut params = self.clone();
        params.cursor = cursor;
        params
    }

    fn get_cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }
}

impl ListResult for ListToolsResult {
    type Item = Tool;

    fn get_items(&self) -> &[Self::Item] {
        &self.tools
    }

    fn get_next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_deref()
    }

    fn with_items_and_cursor(
        items: Vec<Self::Item>,
        next_cursor: Option<String>,
        meta: Option<BTreeMap<String, Value>>,
    ) -> Self {
        Self { tools: items, next_cursor, meta }
    }
}

impl ListRequestParams for ListToolsRequestParams {
    fn with_cursor(&self, cursor: Option<String>) -> Self {
        let mut params = self.clone();
        params.cursor = cursor;
        params
    }

    fn get_cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }
}

impl ListResult for ListResourceTemplatesResult {
    type Item = ResourceTemplate;

    fn get_items(&self) -> &[Self::Item] {
        &self.resource_templates
    }

    fn get_next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_deref()
    }

    fn with_items_and_cursor(
        items: Vec<Self::Item>,
        next_cursor: Option<String>,
        meta: Option<BTreeMap<String, Value>>,
    ) -> Self {
        Self { resource_templates: items, next_cursor, meta }
    }
}

impl ListRequestParams for ListResourceTemplatesRequestParams {
    fn with_cursor(&self, cursor: Option<String>) -> Self {
        let mut params = self.clone();
        params.cursor = cursor;
        params
    }

    fn get_cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }
}
