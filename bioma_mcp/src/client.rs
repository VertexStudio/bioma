use crate::schema::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, CreateMessageRequestParams, CreateMessageResult,
    GetPromptRequestParams, GetPromptResult, Implementation, InitializeRequestParams, InitializeResult,
    InitializedNotificationParams, ListPromptsRequestParams, ListPromptsResult, ListResourceTemplatesRequestParams,
    ListResourceTemplatesResult, ListResourcesRequestParams, ListResourcesResult, ListToolsRequestParams,
    ListToolsResult, ReadResourceRequestParams, ReadResourceResult, Root, RootsListChangedNotificationParams,
    ServerCapabilities,
};
use crate::transport::sse::SseTransport;
use crate::transport::ws::WsTransport;
use crate::transport::{stdio::StdioTransport, Transport, TransportSender, TransportType};
use crate::{ConnectionId, JsonRpcMessage};
use anyhow::Error;
use jsonrpc_core::{MetaIoHandler, Params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StdioConfig {
    pub command: String,
    pub args: Vec<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct ClientConfig {
    pub server: ServerConfig,
}

pub trait ModelContextProtocolClient: Send + Sync + 'static {
    fn get_server_config(&self) -> impl Future<Output = ServerConfig> + Send;
    fn get_capabilities(&self) -> impl Future<Output = ClientCapabilities> + Send;
    fn get_roots(&self) -> impl Future<Output = Vec<Root>> + Send;
    fn on_create_message(&self, params: CreateMessageRequestParams)
        -> impl Future<Output = CreateMessageResult> + Send;
}

type RequestId = u64;
type ResponseSender = oneshot::Sender<Result<serde_json::Value, ClientError>>;
type PendingRequests = Arc<Mutex<HashMap<RequestId, ResponseSender>>>;

struct ServerConnection {
    transport: TransportType,
    transport_sender: TransportSender,
    server_capabilities: Arc<RwLock<Option<ServerCapabilities>>>,
    roots: Arc<RwLock<HashMap<String, Root>>>,
    #[allow(unused)]
    io_handler: MetaIoHandler<()>,
    request_counter: Arc<RwLock<u64>>,
    start_handle: JoinHandle<Result<(), Error>>,
    #[allow(unused)]
    message_handler: JoinHandle<()>,
    pending_requests: PendingRequests,
    #[allow(unused)]
    on_error_rx: mpsc::Receiver<Error>,
    #[allow(unused)]
    on_close_rx: mpsc::Receiver<()>,
    conn_id: ConnectionId,
}

pub struct Client<T: ModelContextProtocolClient> {
    client: Arc<RwLock<T>>,
    connections: HashMap<String, ServerConnection>,
    current_server: Option<String>,
}

impl<T: ModelContextProtocolClient> Client<T> {
    pub async fn new(client: T) -> Result<Self, ClientError> {
        let client_arc = Arc::new(RwLock::new(client));
        let server_config = client_arc.read().await.get_server_config().await;
        let server_name = server_config.name.clone();

        let mut client =
            Self { client: client_arc, connections: HashMap::new(), current_server: Some(server_name.clone()) };

        client.add_server(server_name, server_config).await?;

        Ok(client)
    }

    pub async fn add_server(&mut self, name: String, server_config: ServerConfig) -> Result<(), ClientError> {
        let (on_message_tx, mut on_message_rx) = mpsc::channel::<JsonRpcMessage>(1);
        let (on_error_tx, on_error_rx) = mpsc::channel::<Error>(1);
        let (on_close_tx, on_close_rx) = mpsc::channel::<()>(1);

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

        let conn_id = ConnectionId::new();

        let mut io_handler = MetaIoHandler::default();

        io_handler.add_method_with_meta("sampling/createMessage", {
            let client = self.client.clone();
            move |params: Params, _: ()| {
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
                    info!("Successfully handled createMessage request");
                    Ok(serde_json::to_value(result).map_err(|e| {
                        error!("Failed to serialize createMessage result: {}", e);
                        jsonrpc_core::Error::invalid_params(e.to_string())
                    })?)
                }
            }
        });

        io_handler.add_method_with_meta("roots/list", {
            let client = self.client.clone();
            move |_params: Params, _: ()| {
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

        let transport_sender = transport.sender();
        let conn_id_clone = conn_id.clone();

        let transport_sender_clone = transport_sender.clone();
        let io_handler_clone = io_handler.clone();
        let start_handle =
            transport.start().await.map_err(|e| ClientError::Transport(format!("Start: {}", e).into()))?;

        let pending_requests = Arc::new(Mutex::new(HashMap::<u64, ResponseSender>::new()));
        let pending_requests_clone = pending_requests.clone();

        let message_handler = tokio::spawn({
            let pending_requests = pending_requests_clone;
            async move {
                while let Some(message) = on_message_rx.recv().await {
                    match &message {
                        JsonRpcMessage::Response(jsonrpc_core::Response::Single(output)) => match output {
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
                                        let _ = sender.send(Err(ClientError::Request(
                                            format!("RPC error: {:?}", failure.error).into(),
                                        )));
                                    }
                                }
                            }
                        },
                        JsonRpcMessage::Request(request) => match request {
                            jsonrpc_core::Request::Single(jsonrpc_core::Call::MethodCall(_call)) => {
                                let Some(response) = io_handler_clone.handle_rpc_request(request.clone(), ()).await
                                else {
                                    return;
                                };

                                if let Err(e) =
                                    transport_sender_clone.send(response.into(), conn_id_clone.clone()).await
                                {
                                    error!("Failed to send response: {}", e);
                                }
                            }
                            jsonrpc_core::Request::Single(jsonrpc_core::Call::Notification(notification)) => {
                                info!("Got notification: {:?}", notification);
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }
        });

        let server_connection = ServerConnection {
            transport,
            transport_sender,
            server_capabilities: Arc::new(RwLock::new(None)),
            roots: Arc::new(RwLock::new(HashMap::new())),
            io_handler,
            request_counter: Arc::new(RwLock::new(0)),
            start_handle,
            message_handler,
            pending_requests,
            on_error_rx,
            on_close_rx,
            conn_id,
        };

        self.connections.insert(name.clone(), server_connection);

        if self.current_server.is_none() {
            self.current_server = Some(name);
        }

        Ok(())
    }

    pub fn server(&mut self, name: &str) -> Result<&mut Self, ClientError> {
        if !self.connections.contains_key(name) {
            return Err(ClientError::Request(format!("Server '{}' not found", name).into()));
        }

        self.current_server = Some(name.to_string());
        Ok(self)
    }

    pub fn server_names(&self) -> Vec<String> {
        self.connections.keys().cloned().collect()
    }

    fn get_current_server_name(&self) -> Result<String, ClientError> {
        self.current_server.clone().ok_or_else(|| ClientError::Request("No active server connection".into()))
    }

    pub async fn update_roots(&mut self, roots: HashMap<String, Root>) -> Result<(), ClientError> {
        let server_name = self.get_current_server_name()?;

        if !self.connections.contains_key(&server_name) {
            return Err(ClientError::Request(format!("Server '{}' not found", server_name).into()));
        }

        {
            let conn = self.connections.get_mut(&server_name).unwrap();
            let mut old_roots = conn.roots.write().await;
            *old_roots = roots.clone();
        }

        let params = RootsListChangedNotificationParams { meta: None };
        self.notify(&server_name, "notifications/roots/list_changed".to_string(), serde_json::to_value(params)?)
            .await?;

        Ok(())
    }

    pub async fn initialize(&mut self, client_info: Implementation) -> Result<InitializeResult, ClientError> {
        let server_name = self.get_current_server_name()?;

        if !self.connections.contains_key(&server_name) {
            return Err(ClientError::Request(format!("Server '{}' not found", server_name).into()));
        }

        let params = InitializeRequestParams {
            protocol_version: "2024-11-05".to_string(),
            capabilities: self.client.read().await.get_capabilities().await,
            client_info,
        };

        let response = self.request(&server_name, "initialize".to_string(), serde_json::to_value(params)?).await?;
        let result: InitializeResult = serde_json::from_value(response)?;

        {
            let conn = self.connections.get_mut(&server_name).unwrap();
            let mut server_capabilities = conn.server_capabilities.write().await;
            *server_capabilities = Some(result.capabilities.clone());
        }

        Ok(result)
    }

    pub async fn initialized(&mut self) -> Result<(), ClientError> {
        let server_name = self.get_current_server_name()?;
        let params = InitializedNotificationParams { meta: None };
        self.notify(&server_name, "notifications/initialized".to_string(), serde_json::to_value(params)?).await?;
        Ok(())
    }

    pub async fn list_resources(
        &mut self,
        params: Option<ListResourcesRequestParams>,
    ) -> Result<ListResourcesResult, ClientError> {
        let server_name = self.get_current_server_name()?;
        debug!("Server {} - Sending resources/list request", server_name);
        let response = self.request(&server_name, "resources/list".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn read_resource(
        &mut self,
        params: ReadResourceRequestParams,
    ) -> Result<ReadResourceResult, ClientError> {
        let server_name = self.get_current_server_name()?;
        debug!("Server {} - Sending resources/read request", server_name);
        let response = self.request(&server_name, "resources/read".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn list_resource_templates(
        &mut self,
        params: Option<ListResourceTemplatesRequestParams>,
    ) -> Result<ListResourceTemplatesResult, ClientError> {
        let server_name = self.get_current_server_name()?;
        debug!("Server {} - Sending resources/templates/list request", server_name);
        let response =
            self.request(&server_name, "resources/templates/list".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn subscribe_resource(&mut self, uri: String) -> Result<(), ClientError> {
        let server_name = self.get_current_server_name()?;
        debug!("Server {} - Sending resources/subscribe request for {}", server_name, uri);
        let params = serde_json::json!({ "uri": uri });
        let _response = self.request(&server_name, "resources/subscribe".to_string(), params).await?;
        Ok(())
    }

    pub async fn unsubscribe_resource(&mut self, uri: String) -> Result<(), ClientError> {
        let server_name = self.get_current_server_name()?;
        debug!("Server {} - Sending resources/unsubscribe request for {}", server_name, uri);
        let params = serde_json::json!({ "uri": uri });
        let _response = self.request(&server_name, "resources/unsubscribe".to_string(), params).await?;
        Ok(())
    }

    pub async fn list_prompts(
        &mut self,
        params: Option<ListPromptsRequestParams>,
    ) -> Result<ListPromptsResult, ClientError> {
        let server_name = self.get_current_server_name()?;
        debug!("Server {} - Sending prompts/list request", server_name);
        let response = self.request(&server_name, "prompts/list".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn get_prompt(&mut self, params: GetPromptRequestParams) -> Result<GetPromptResult, ClientError> {
        let server_name = self.get_current_server_name()?;
        debug!("Server {} - Sending prompts/get request", server_name);
        let response = self.request(&server_name, "prompts/get".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn list_tools(&mut self, params: Option<ListToolsRequestParams>) -> Result<ListToolsResult, ClientError> {
        let server_name = self.get_current_server_name()?;
        debug!("Server {} - Sending tools/list request", server_name);
        let response = self.request(&server_name, "tools/list".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn call_tool(&mut self, params: CallToolRequestParams) -> Result<CallToolResult, ClientError> {
        let server_name = self.get_current_server_name()?;
        debug!("Server {} - Sending tools/call request", server_name);
        let response = self.request(&server_name, "tools/call".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn add_root(&mut self, root: Root, meta: Option<BTreeMap<String, Value>>) -> Result<(), ClientError> {
        let server_name = self.get_current_server_name()?;

        if !self.connections.contains_key(&server_name) {
            return Err(ClientError::Request(format!("Server '{}' not found", server_name).into()));
        }

        let capabilities = self.client.read().await.get_capabilities().await;
        let supports_root_notifications = capabilities.roots.map_or(false, |roots| roots.list_changed.unwrap_or(false));

        let should_notify = {
            let conn = self.connections.get_mut(&server_name).unwrap();
            let mut roots = conn.roots.write().await;
            let root_is_new = roots.insert(root.uri.clone(), root.clone()).is_none();
            root_is_new && supports_root_notifications
        };

        if should_notify {
            let params = RootsListChangedNotificationParams { meta };
            self.notify(&server_name, "notifications/rootsListChanged".to_string(), serde_json::to_value(params)?)
                .await?;
        }

        Ok(())
    }

    pub async fn remove_root(&mut self, uri: String, meta: Option<BTreeMap<String, Value>>) -> Result<(), ClientError> {
        let server_name = self.get_current_server_name()?;

        if !self.connections.contains_key(&server_name) {
            return Err(ClientError::Request(format!("Server '{}' not found", server_name).into()));
        }

        let capabilities = self.client.read().await.get_capabilities().await;
        let supports_root_notifications = capabilities.roots.map_or(false, |roots| roots.list_changed.unwrap_or(false));

        let should_notify = {
            let conn = self.connections.get_mut(&server_name).unwrap();
            let mut roots = conn.roots.write().await;
            let root_existed = roots.remove(&uri).is_some();
            root_existed && supports_root_notifications
        };

        if should_notify {
            let params = RootsListChangedNotificationParams { meta };
            self.notify(&server_name, "notifications/rootsListChanged".to_string(), serde_json::to_value(params)?)
                .await?;
        }

        Ok(())
    }

    pub async fn close(&mut self) -> Result<(), ClientError> {
        let server_name = self.get_current_server_name()?;

        if let Some(conn) = self.connections.get_mut(&server_name) {
            conn.transport.close().await.map_err(|e| ClientError::Transport(format!("Close: {}", e).into()))?;
            conn.start_handle.abort();
            Ok(())
        } else {
            Err(ClientError::Request(format!("Server '{}' not found", server_name).into()))
        }
    }

    pub async fn remove_server(&mut self, name: &str) -> Result<(), ClientError> {
        if !self.connections.contains_key(name) {
            return Err(ClientError::Request(format!("Server '{}' not found", name).into()));
        }

        if let Some(current) = &self.current_server {
            if current == name {
                self.current_server = self.connections.keys().find(|&k| k != name).map(|k| k.clone());
            }
        }

        if let Some(mut conn) = self.connections.remove(name) {
            conn.transport.close().await.map_err(|e| ClientError::Transport(format!("Close: {}", e).into()))?;
            conn.start_handle.abort();
        }

        Ok(())
    }

    pub async fn close_all(&mut self) -> Result<(), ClientError> {
        let mut errors = Vec::new();

        for (name, conn) in &mut self.connections {
            if let Err(e) = conn.transport.close().await {
                errors.push(format!("Failed to close connection to '{}': {}", name, e));
            }
            conn.start_handle.abort();
        }

        self.connections.clear();
        self.current_server = None;

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ClientError::Request(errors.join(", ").into()))
        }
    }

    async fn request(
        &mut self,
        server_name: &str,
        method: String,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        let conn = self
            .connections
            .get_mut(server_name)
            .ok_or_else(|| ClientError::Request(format!("Server '{}' not found", server_name).into()))?;

        let mut counter = conn.request_counter.write().await;
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
            let mut pending = conn.pending_requests.lock().await;
            pending.insert(id, response_tx);
        }

        let conn_id = conn.conn_id.clone();

        if let Err(e) = conn.transport_sender.send(request.into(), conn_id).await {
            let mut pending = conn.pending_requests.lock().await;
            pending.remove(&id);
            return Err(ClientError::Transport(format!("Send: {}", e).into()));
        }

        let timeout = self.client.read().await.get_server_config().await.request_timeout;

        match tokio::time::timeout(std::time::Duration::from_secs(timeout), response_rx).await {
            Ok(response) => match response {
                Ok(result) => result,
                Err(_) => Err(ClientError::Request("Response channel closed".into())),
            },
            Err(_) => {
                let mut pending = conn.pending_requests.lock().await;
                pending.remove(&id);
                Err(ClientError::Request("Request timed out".into()))
            }
        }
    }

    async fn notify(
        &mut self,
        server_name: &str,
        method: String,
        params: serde_json::Value,
    ) -> Result<(), ClientError> {
        let conn = self
            .connections
            .get_mut(server_name)
            .ok_or_else(|| ClientError::Request(format!("Server '{}' not found", server_name).into()))?;

        let notification = jsonrpc_core::Notification {
            jsonrpc: Some(jsonrpc_core::Version::V2),
            method,
            params: Params::Map(params.as_object().cloned().unwrap_or_default()),
        };

        let conn_id = conn.conn_id.clone();

        conn.transport_sender
            .send(notification.into(), conn_id)
            .await
            .map_err(|e| ClientError::Transport(format!("Send: {}", e).into()))
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
}

impl<T: ModelContextProtocolClient> std::fmt::Debug for Client<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("servers", &self.server_names())
            .field("current_server", &self.current_server)
            .finish()
    }
}
