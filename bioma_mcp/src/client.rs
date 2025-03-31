use crate::schema::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, CreateMessageRequestParams, CreateMessageResult,
    GetPromptRequestParams, GetPromptResult, Implementation, InitializeRequestParams, InitializeResult,
    InitializedNotificationParams, ListPromptsRequestParams, ListPromptsResult, ListResourceTemplatesRequestParams,
    ListResourceTemplatesResult, ListResourcesRequestParams, ListResourcesResult, ListToolsRequestParams,
    ListToolsResult, Prompt, ReadResourceRequestParams, ReadResourceResult, Resource, ResourceTemplate, Root,
    RootsListChangedNotificationParams, ServerCapabilities, Tool,
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
use tracing::{error, info, warn};

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

#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder, Default)]
pub struct ClientConfig {
    pub name: String,
    pub servers: Vec<ServerConfig>,
}

pub trait ModelContextProtocolClient: Send + Sync + 'static {
    fn get_server_configs(&self) -> impl Future<Output = Vec<ServerConfig>> + Send;
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
    io_handler: MetaIoHandler<()>,
    roots: Arc<RwLock<HashMap<String, Root>>>,
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
            let client = client.clone();
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

        let mut client =
            Self { client, connections: HashMap::new(), io_handler, roots: Arc::new(RwLock::new(HashMap::new())) };

        for config in server_configs {
            let name = config.name.clone();
            client.add_server(name, config).await?;
        }

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

        let conn_id = ConnectionId::new(Some(server_config.name.clone()));
        let transport_sender = transport.sender();
        let conn_id_clone = conn_id.clone();

        let transport_sender_clone = transport_sender.clone();
        let io_handler_clone = self.io_handler.clone();
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
            request_counter: Arc::new(RwLock::new(0)),
            start_handle,
            message_handler,
            pending_requests,
            on_error_rx,
            on_close_rx,
            conn_id,
        };

        self.connections.insert(name, server_connection);
        Ok(())
    }

    fn list_servers(&self) -> Vec<String> {
        self.connections.keys().cloned().collect()
    }

    async fn request(
        connection: &mut ServerConnection,
        method: String,
        params: serde_json::Value,
        client: Arc<RwLock<T>>,
    ) -> Result<serde_json::Value, ClientError> {
        let mut counter = connection.request_counter.write().await;
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
            let mut pending = connection.pending_requests.lock().await;
            pending.insert(id, response_tx);
        }

        let conn_id = connection.conn_id.clone();

        if let Err(e) = connection.transport_sender.send(request.into(), conn_id).await {
            let mut pending = connection.pending_requests.lock().await;
            pending.remove(&id);
            return Err(ClientError::Transport(format!("Send: {}", e).into()));
        }

        let timeout = client
            .read()
            .await
            .get_server_configs()
            .await
            .iter()
            .find(|cfg| connection.conn_id.contains(&cfg.name))
            .map(|cfg| cfg.request_timeout)
            .unwrap_or(5);

        match tokio::time::timeout(std::time::Duration::from_secs(timeout), response_rx).await {
            Ok(response) => match response {
                Ok(result) => result,
                Err(_) => Err(ClientError::Request("Response channel closed".into())),
            },
            Err(_) => {
                let mut pending = connection.pending_requests.lock().await;
                pending.remove(&id);
                Err(ClientError::Request("Request timed out".into()))
            }
        }
    }

    async fn notify(
        connection: &mut ServerConnection,
        method: String,
        params: serde_json::Value,
    ) -> Result<(), ClientError> {
        let notification = jsonrpc_core::Notification {
            jsonrpc: Some(jsonrpc_core::Version::V2),
            method,
            params: Params::Map(params.as_object().cloned().unwrap_or_default()),
        };

        let conn_id = connection.conn_id.clone();

        connection
            .transport_sender
            .send(notification.into(), conn_id)
            .await
            .map_err(|e| ClientError::Transport(format!("Send: {}", e).into()))
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

            match Self::request(connection, "initialize".to_string(), serde_json::to_value(params)?, client.clone())
                .await
            {
                Ok(response) => match serde_json::from_value::<InitializeResult>(response) {
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
                    errors.push(format!("Failed to initialize '{}': {:?}", server_name, e));
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
                Self::notify(connection, "notifications/initialized".to_string(), serde_json::to_value(params)?).await
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
    ) -> Result<ListResourcesResult, ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let mut all_resources = Vec::new();
        let mut errors = Vec::new();
        let client = self.client.clone();

        for (server_name, connection) in &mut self.connections {
            match Self::request(
                connection,
                "resources/list".to_string(),
                serde_json::to_value(params.clone())?,
                client.clone(),
            )
            .await
            {
                Ok(response) => match serde_json::from_value::<ListResourcesResult>(response) {
                    Ok(result) => {
                        all_resources.extend(result.resources);
                    }
                    Err(e) => {
                        errors.push(format!("Error deserializing result from '{}': {:?}", server_name, e));
                    }
                },
                Err(e) => {
                    errors.push(format!("Error from '{}': {:?}", server_name, e));
                }
            }
        }

        if all_resources.is_empty() && !errors.is_empty() {
            Err(ClientError::Request(format!("All servers failed: {}", errors.join(", ")).into()))
        } else {
            Ok(ListResourcesResult { resources: all_resources, meta: None, next_cursor: None })
        }
    }

    pub async fn read_resource(
        &mut self,
        params: ReadResourceRequestParams,
    ) -> Result<ReadResourceResult, ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let mut errors = Vec::new();
        let client = self.client.clone();

        for (server_name, connection) in &mut self.connections {
            match Self::request(
                connection,
                "resources/read".to_string(),
                serde_json::to_value(params.clone())?,
                client.clone(),
            )
            .await
            {
                Ok(response) => match serde_json::from_value::<ReadResourceResult>(response) {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        errors.push(format!("Error deserializing result from '{}': {:?}", server_name, e));
                    }
                },
                Err(e) => {
                    errors.push(format!("Error from '{}': {:?}", server_name, e));
                }
            }
        }

        Err(ClientError::Request(format!("Unable to read resource from any server: {}", errors.join(", ")).into()))
    }

    pub async fn list_resource_templates(
        &mut self,
        params: Option<ListResourceTemplatesRequestParams>,
    ) -> Result<ListResourceTemplatesResult, ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let mut all_templates = Vec::new();
        let mut errors = Vec::new();
        let client = self.client.clone();

        for (server_name, connection) in &mut self.connections {
            match Self::request(
                connection,
                "resources/templates/list".to_string(),
                serde_json::to_value(params.clone())?,
                client.clone(),
            )
            .await
            {
                Ok(response) => match serde_json::from_value::<ListResourceTemplatesResult>(response) {
                    Ok(result) => {
                        all_templates.extend(result.resource_templates);
                    }
                    Err(e) => {
                        errors.push(format!("Error deserializing result from '{}': {:?}", server_name, e));
                    }
                },
                Err(e) => {
                    errors.push(format!("Error from '{}': {:?}", server_name, e));
                }
            }
        }

        if all_templates.is_empty() && !errors.is_empty() {
            Err(ClientError::Request(format!("All servers failed: {}", errors.join(", ")).into()))
        } else {
            Ok(ListResourceTemplatesResult { resource_templates: all_templates, meta: None, next_cursor: None })
        }
    }

    pub async fn subscribe_resource(&mut self, uri: String) -> Result<(), ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let mut successful = 0;
        let mut errors = Vec::new();
        let client = self.client.clone();

        for (server_name, connection) in &mut self.connections {
            let params = serde_json::json!({ "uri": uri.clone() });
            match Self::request(connection, "resources/subscribe".to_string(), params, client.clone()).await {
                Ok(_) => {
                    successful += 1;
                }
                Err(e) => {
                    errors.push(format!("Error from '{}': {:?}", server_name, e));
                }
            }
        }

        if successful > 0 {
            if !errors.is_empty() {
                warn!("Some servers failed to subscribe: {}", errors.join(", "));
            }
            Ok(())
        } else {
            Err(ClientError::Request(format!("Failed to subscribe on all servers: {}", errors.join(", ")).into()))
        }
    }

    pub async fn unsubscribe_resource(&mut self, uri: String) -> Result<(), ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let mut successful = 0;
        let mut errors = Vec::new();
        let client = self.client.clone();

        for (server_name, connection) in &mut self.connections {
            let params = serde_json::json!({ "uri": uri.clone() });
            match Self::request(connection, "resources/unsubscribe".to_string(), params, client.clone()).await {
                Ok(_) => {
                    successful += 1;
                }
                Err(e) => {
                    errors.push(format!("Error from '{}': {:?}", server_name, e));
                }
            }
        }

        if successful > 0 {
            if !errors.is_empty() {
                warn!("Some servers failed to unsubscribe: {}", errors.join(", "));
            }
            Ok(())
        } else {
            Err(ClientError::Request(format!("Failed to unsubscribe on all servers: {}", errors.join(", ")).into()))
        }
    }

    pub async fn list_prompts(
        &mut self,
        params: Option<ListPromptsRequestParams>,
    ) -> Result<ListPromptsResult, ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let mut all_prompts = Vec::new();
        let mut errors = Vec::new();
        let client = self.client.clone();

        for (server_name, connection) in &mut self.connections {
            match Self::request(
                connection,
                "prompts/list".to_string(),
                serde_json::to_value(params.clone())?,
                client.clone(),
            )
            .await
            {
                Ok(response) => match serde_json::from_value::<ListPromptsResult>(response) {
                    Ok(result) => {
                        all_prompts.extend(result.prompts);
                    }
                    Err(e) => {
                        errors.push(format!("Error deserializing result from '{}': {:?}", server_name, e));
                    }
                },
                Err(e) => {
                    errors.push(format!("Error from '{}': {:?}", server_name, e));
                }
            }
        }

        if all_prompts.is_empty() && !errors.is_empty() {
            Err(ClientError::Request(format!("All servers failed: {}", errors.join(", ")).into()))
        } else {
            Ok(ListPromptsResult { prompts: all_prompts, meta: None, next_cursor: None })
        }
    }

    pub async fn get_prompt(&mut self, params: GetPromptRequestParams) -> Result<GetPromptResult, ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let mut errors = Vec::new();
        let client = self.client.clone();

        for (server_name, connection) in &mut self.connections {
            match Self::request(
                connection,
                "prompts/get".to_string(),
                serde_json::to_value(params.clone())?,
                client.clone(),
            )
            .await
            {
                Ok(response) => match serde_json::from_value::<GetPromptResult>(response) {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        errors.push(format!("Error deserializing result from '{}': {:?}", server_name, e));
                    }
                },
                Err(e) => {
                    errors.push(format!("Error from '{}': {:?}", server_name, e));
                }
            }
        }

        Err(ClientError::Request(format!("Unable to get prompt from any server: {}", errors.join(", ")).into()))
    }

    pub async fn list_tools(&mut self, params: Option<ListToolsRequestParams>) -> Result<ListToolsResult, ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let mut all_tools = Vec::new();
        let mut errors = Vec::new();
        let client = self.client.clone();

        for (server_name, connection) in &mut self.connections {
            match Self::request(
                connection,
                "tools/list".to_string(),
                serde_json::to_value(params.clone())?,
                client.clone(),
            )
            .await
            {
                Ok(response) => match serde_json::from_value::<ListToolsResult>(response) {
                    Ok(result) => {
                        all_tools.extend(result.tools);
                    }
                    Err(e) => {
                        errors.push(format!("Error deserializing result from '{}': {:?}", server_name, e));
                    }
                },
                Err(e) => {
                    errors.push(format!("Error from '{}': {:?}", server_name, e));
                }
            }
        }

        if all_tools.is_empty() && !errors.is_empty() {
            Err(ClientError::Request(format!("All servers failed: {}", errors.join(", ")).into()))
        } else {
            Ok(ListToolsResult { tools: all_tools, meta: None, next_cursor: None })
        }
    }

    pub async fn call_tool(&mut self, params: CallToolRequestParams) -> Result<CallToolResult, ClientError> {
        if self.connections.is_empty() {
            return Err(ClientError::Request("No server connections available".into()));
        }

        let mut errors = Vec::new();
        let client = self.client.clone();

        for (server_name, connection) in &mut self.connections {
            match Self::request(
                connection,
                "tools/call".to_string(),
                serde_json::to_value(params.clone())?,
                client.clone(),
            )
            .await
            {
                Ok(response) => match serde_json::from_value::<CallToolResult>(response) {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        errors.push(format!("Error deserializing result from '{}': {:?}", server_name, e));
                    }
                },
                Err(e) => {
                    errors.push(format!("Error from '{}': {:?}", server_name, e));
                }
            }
        }

        Err(ClientError::Request(format!("Unable to call tool on any server: {}", errors.join(", ")).into()))
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
                Self::notify(connection, "notifications/roots/list_changed".to_string(), serde_json::to_value(params)?)
                    .await
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
                Self::notify(connection, "notifications/roots/list_changed".to_string(), serde_json::to_value(params)?)
                    .await
            {
                errors.push(format!("Failed to notify root removal on '{}': {:?}", server_name, e));
            }
        }

        if !errors.is_empty() {
            warn!("Failed to remove root from some servers: {}", errors.join(", "));
        }

        Ok(())
    }

    pub async fn list_all_resources(
        &mut self,
        params: Option<ListResourcesRequestParams>,
    ) -> Result<Vec<Resource>, ClientError> {
        let mut all_resources = Vec::new();
        let mut cursor = None;

        loop {
            let mut req_params = params.clone().unwrap_or_default();
            req_params.cursor = cursor;

            let result = self.list_resources(Some(req_params)).await?;
            all_resources.extend(result.resources);

            if let Some(next_cursor) = result.next_cursor {
                cursor = Some(next_cursor);
            } else {
                break;
            }
        }

        Ok(all_resources)
    }

    pub async fn list_all_prompts(
        &mut self,
        params: Option<ListPromptsRequestParams>,
    ) -> Result<Vec<Prompt>, ClientError> {
        let mut all_prompts = Vec::new();
        let mut cursor = None;

        loop {
            let mut req_params = params.clone().unwrap_or_default();
            req_params.cursor = cursor;

            let result = self.list_prompts(Some(req_params)).await?;
            all_prompts.extend(result.prompts);

            if let Some(next_cursor) = result.next_cursor {
                cursor = Some(next_cursor);
            } else {
                break;
            }
        }

        Ok(all_prompts)
    }

    pub async fn list_all_tools(&mut self, params: Option<ListToolsRequestParams>) -> Result<Vec<Tool>, ClientError> {
        let mut all_tools = Vec::new();
        let mut cursor = None;

        loop {
            let mut req_params = params.clone().unwrap_or_default();
            req_params.cursor = cursor;

            let result = self.list_tools(Some(req_params)).await?;
            all_tools.extend(result.tools);

            if let Some(next_cursor) = result.next_cursor {
                cursor = Some(next_cursor);
            } else {
                break;
            }
        }

        Ok(all_tools)
    }

    pub async fn list_all_resource_templates(
        &mut self,
        params: Option<ListResourceTemplatesRequestParams>,
    ) -> Result<Vec<ResourceTemplate>, ClientError> {
        let mut all_templates = Vec::new();
        let mut cursor = None;

        loop {
            let mut req_params = params.clone().unwrap_or_default();
            req_params.cursor = cursor;

            let result = self.list_resource_templates(Some(req_params)).await?;
            all_templates.extend(result.resource_templates);

            if let Some(next_cursor) = result.next_cursor {
                cursor = Some(next_cursor);
            } else {
                break;
            }
        }

        Ok(all_templates)
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
            connection.start_handle.abort();
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ClientError::Request(errors.join(", ").into()))
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
}

impl<T: ModelContextProtocolClient> std::fmt::Debug for Client<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").field("servers", &self.list_servers()).finish()
    }
}

pub struct ResourcesIterator<'a, T: ModelContextProtocolClient> {
    client: &'a mut Client<T>,
    params: Option<ListResourcesRequestParams>,
    next_cursor: Option<String>,
    done: bool,
}

impl<'a, T: ModelContextProtocolClient> ResourcesIterator<'a, T> {
    pub fn new(client: &'a mut Client<T>, params: Option<ListResourcesRequestParams>) -> Self {
        Self { client, params, next_cursor: None, done: false }
    }

    pub async fn next(&mut self) -> Option<Result<Vec<Resource>, ClientError>> {
        if self.done {
            return None;
        }

        let mut params = self.params.clone().unwrap_or_default();
        if let Some(cursor) = &self.next_cursor {
            params.cursor = Some(cursor.clone());
        }

        let result = self.client.list_resources(Some(params)).await;

        match result {
            Ok(response) => {
                let items = response.resources;
                self.next_cursor = response.next_cursor;
                self.done = self.next_cursor.is_none();
                Some(Ok(items))
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

pub struct PromptsIterator<'a, T: ModelContextProtocolClient> {
    client: &'a mut Client<T>,
    params: Option<ListPromptsRequestParams>,
    next_cursor: Option<String>,
    done: bool,
}

impl<'a, T: ModelContextProtocolClient> PromptsIterator<'a, T> {
    pub fn new(client: &'a mut Client<T>, params: Option<ListPromptsRequestParams>) -> Self {
        Self { client, params, next_cursor: None, done: false }
    }

    pub async fn next(&mut self) -> Option<Result<Vec<Prompt>, ClientError>> {
        if self.done {
            return None;
        }

        let mut params = self.params.clone().unwrap_or_default();
        if let Some(cursor) = &self.next_cursor {
            params.cursor = Some(cursor.clone());
        }

        let result = self.client.list_prompts(Some(params)).await;

        match result {
            Ok(response) => {
                let items = response.prompts;
                self.next_cursor = response.next_cursor;
                self.done = self.next_cursor.is_none();
                Some(Ok(items))
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

pub struct ToolsIterator<'a, T: ModelContextProtocolClient> {
    client: &'a mut Client<T>,
    params: Option<ListToolsRequestParams>,
    next_cursor: Option<String>,
    done: bool,
}

impl<'a, T: ModelContextProtocolClient> ToolsIterator<'a, T> {
    pub fn new(client: &'a mut Client<T>, params: Option<ListToolsRequestParams>) -> Self {
        Self { client, params, next_cursor: None, done: false }
    }

    pub async fn next(&mut self) -> Option<Result<Vec<Tool>, ClientError>> {
        if self.done {
            return None;
        }

        let mut params = self.params.clone().unwrap_or_default();
        if let Some(cursor) = &self.next_cursor {
            params.cursor = Some(cursor.clone());
        }

        let result = self.client.list_tools(Some(params)).await;

        match result {
            Ok(response) => {
                let items = response.tools;
                self.next_cursor = response.next_cursor;
                self.done = self.next_cursor.is_none();
                Some(Ok(items))
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

pub struct ResourceTemplatesIterator<'a, T: ModelContextProtocolClient> {
    client: &'a mut Client<T>,
    params: Option<ListResourceTemplatesRequestParams>,
    next_cursor: Option<String>,
    done: bool,
}

impl<'a, T: ModelContextProtocolClient> ResourceTemplatesIterator<'a, T> {
    pub fn new(client: &'a mut Client<T>, params: Option<ListResourceTemplatesRequestParams>) -> Self {
        Self { client, params, next_cursor: None, done: false }
    }

    pub async fn next(&mut self) -> Option<Result<Vec<ResourceTemplate>, ClientError>> {
        if self.done {
            return None;
        }

        let mut params = self.params.clone().unwrap_or_default();
        if let Some(cursor) = &self.next_cursor {
            params.cursor = Some(cursor.clone());
        }

        let result = self.client.list_resource_templates(Some(params)).await;

        match result {
            Ok(response) => {
                let items = response.resource_templates;
                self.next_cursor = response.next_cursor;
                self.done = self.next_cursor.is_none();
                Some(Ok(items))
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

impl<T: ModelContextProtocolClient> Client<T> {
    pub fn iter_resources<'a>(&'a mut self, params: Option<ListResourcesRequestParams>) -> ResourcesIterator<'a, T> {
        ResourcesIterator::new(self, params)
    }

    pub fn iter_prompts<'a>(&'a mut self, params: Option<ListPromptsRequestParams>) -> PromptsIterator<'a, T> {
        PromptsIterator::new(self, params)
    }

    pub fn iter_tools<'a>(&'a mut self, params: Option<ListToolsRequestParams>) -> ToolsIterator<'a, T> {
        ToolsIterator::new(self, params)
    }

    pub fn iter_resource_templates<'a>(
        &'a mut self,
        params: Option<ListResourceTemplatesRequestParams>,
    ) -> ResourceTemplatesIterator<'a, T> {
        ResourceTemplatesIterator::new(self, params)
    }
}
