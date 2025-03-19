use crate::sampling::SamplingHandler;
use crate::schema::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, CreateMessageRequest, CreateMessageResult,
    GetPromptRequestParams, GetPromptResult, Implementation, InitializeRequestParams, InitializeResult,
    InitializedNotificationParams, ListPromptsRequestParams, ListPromptsResult, ListResourcesRequestParams,
    ListResourcesResult, ListToolsRequestParams, ListToolsResult, ReadResourceRequestParams, ReadResourceResult,
    ServerCapabilities,
};
use crate::transport::sse::SseTransport;
use crate::transport::ws::WsTransport;
use crate::transport::{stdio::StdioTransport, Transport, TransportType};
use crate::JsonRpcMessage;
use anyhow::Error;
use bioma_actor::prelude::*;
use jsonrpc_core::{MetaIoHandler, Metadata, Params};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
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
    #[serde(default = "default_version")]
    #[builder(default = default_version())]
    pub version: String,
    #[serde(flatten)]
    pub transport: TransportConfig,
    #[serde(default = "default_request_timeout")]
    #[builder(default = default_request_timeout())]
    pub request_timeout: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct ClientConfig {
    pub server: ServerConfig,
    #[serde(default)]
    #[builder(default)]
    pub capabilities: ClientCapabilities,
}

fn default_version() -> String {
    "0.1.0".to_string()
}

fn default_request_timeout() -> u64 {
    5
}

#[derive(Clone)]
pub struct ClientMetadata {}

impl Metadata for ClientMetadata {}

type RequestId = u64;
type ResponseSender = oneshot::Sender<Result<serde_json::Value, ModelContextProtocolClientError>>;
type PendingRequests = Arc<Mutex<HashMap<RequestId, ResponseSender>>>;

pub struct ModelContextProtocolClient {
    server: ServerConfig,
    transport: TransportType,
    pub server_capabilities: Arc<RwLock<Option<ServerCapabilities>>>,
    request_counter: Arc<RwLock<u64>>,
    start_handle: JoinHandle<Result<(), Error>>,
    #[allow(unused)]
    message_handler: JoinHandle<()>,
    pending_requests: PendingRequests,
    #[allow(unused)]
    on_error_rx: mpsc::Receiver<Error>,
    #[allow(unused)]
    on_close_rx: mpsc::Receiver<()>,
    client_capabilities: ClientCapabilities,
}

impl ModelContextProtocolClient {
    pub async fn new(
        server: ServerConfig,
        mut capabilities: ClientCapabilities,
        sampling_handler: Option<Arc<dyn SamplingHandler + Send + Sync>>,
    ) -> Result<Self, ModelContextProtocolClientError> {
        // Synchronize capabilities with handlers
        if sampling_handler.is_some() && capabilities.sampling.is_none() {
            capabilities.sampling = Some(BTreeMap::new());
        } else if sampling_handler.is_none() && capabilities.sampling.is_some() {
            return Err(ModelContextProtocolClientError::Configuration(
                "Sampling capability enabled but no handler provided".into(),
            ));
        }

        let (on_message_tx, mut on_message_rx) = mpsc::channel::<JsonRpcMessage>(1);
        let (on_error_tx, on_error_rx) = mpsc::channel::<Error>(1);
        let (on_close_tx, on_close_rx) = mpsc::channel::<()>(1);

        let mut transport = match &server.transport {
            TransportConfig::Stdio(config) => {
                let transport = StdioTransport::new_client(&config, on_message_tx, on_error_tx, on_close_tx).await;
                let transport = match transport {
                    Ok(transport) => transport,
                    Err(e) => {
                        return Err(ModelContextProtocolClientError::Transport(format!("Client new: {}", e).into()))
                    }
                };
                TransportType::Stdio(transport)
            }
            TransportConfig::Sse(config) => {
                let transport = SseTransport::new_client(config, on_message_tx, on_error_tx, on_close_tx);
                let transport = match transport {
                    Ok(transport) => transport,
                    Err(e) => {
                        return Err(ModelContextProtocolClientError::Transport(format!("Client new: {}", e).into()))
                    }
                };
                TransportType::Sse(transport)
            }
            TransportConfig::Ws(config) => {
                let transport = WsTransport::new_client(config, on_message_tx, on_error_tx, on_close_tx);
                let transport = match transport {
                    Ok(transport) => transport,
                    Err(e) => {
                        return Err(ModelContextProtocolClientError::Transport(format!("Client new: {}", e).into()))
                    }
                };
                TransportType::Ws(transport)
            }
        };

        // Start transport once during initialization
        let start_handle = transport
            .start()
            .await
            .map_err(|e| ModelContextProtocolClientError::Transport(format!("Start: {}", e).into()))?;

        let pending_requests = Arc::new(Mutex::new(HashMap::<u64, ResponseSender>::new()));
        let pending_requests_clone = pending_requests.clone();

        // Create a request handler for server-initiated requests
        let mut request_handler = MetaIoHandler::default();

        // Register sampling handler if available
        if let Some(handler) = &sampling_handler {
            request_handler.add_method_with_meta("sampling/createMessage", {
                let handler = handler.clone();
                move |params: Params, _meta: ClientMetadata| {
                    let handler = handler.clone();
                    async move {
                        let request: CreateMessageRequest = params.parse().map_err(|e| {
                            error!("Failed to parse sampling parameters: {}", e);
                            jsonrpc_core::Error::invalid_params(e.to_string())
                        })?;

                        match handler.create_message_boxed(request).await {
                            Ok(result) => {
                                let response = CreateMessageResult {
                                    content: result.content,
                                    model: result.model,
                                    role: result.role,
                                    stop_reason: result.stop_reason,
                                    meta: result.meta,
                                };
                                Ok(serde_json::to_value(response).map_err(|e| {
                                    error!("Failed to serialize sampling result: {}", e);
                                    jsonrpc_core::Error::internal_error()
                                })?)
                            }
                            Err(e) => {
                                error!("Sampling failed: {}", e);
                                Err(jsonrpc_core::Error::internal_error())
                            }
                        }
                    }
                }
            });
        }

        // TODO: Register roots handler when implemented

        // Create message handler task
        let message_handler = tokio::spawn({
            let pending_requests = pending_requests_clone;
            let request_handler = Arc::new(request_handler);
            let mut transport_clone = transport.clone();

            async move {
                while let Some(message) = on_message_rx.recv().await {
                    match message {
                        JsonRpcMessage::Response(jsonrpc_core::Response::Single(output)) => match output {
                            jsonrpc_core::Output::Success(success) => {
                                if let jsonrpc_core::Id::Num(id) = success.id {
                                    let mut requests = pending_requests.lock().await;
                                    if let Some(sender) = requests.remove(&id) {
                                        let _ = sender.send(Ok(success.result));
                                    }
                                }
                            }
                            jsonrpc_core::Output::Failure(failure) => {
                                if let jsonrpc_core::Id::Num(id) = failure.id {
                                    let mut requests = pending_requests.lock().await;
                                    if let Some(sender) = requests.remove(&id) {
                                        let _ = sender.send(Err(ModelContextProtocolClientError::Request(
                                            format!("RPC error: {:?}", failure.error).into(),
                                        )));
                                    }
                                }
                            }
                        },
                        JsonRpcMessage::Request(request) => {
                            if let Some(response) =
                                request_handler.handle_rpc_request(request.clone(), ClientMetadata {}).await
                            {
                                if let Err(e) = transport_clone.send(response.into(), serde_json::Value::Null).await {
                                    error!("Failed to send response to server-initiated request: {}", e);
                                }
                            }
                        }
                        _ => {} // Handle other message types if needed
                    }
                }
            }
        });

        Ok(Self {
            server,
            transport,
            server_capabilities: Arc::new(RwLock::new(None)),
            request_counter: Arc::new(RwLock::new(0)),
            start_handle,
            message_handler,
            pending_requests,
            on_error_rx,
            on_close_rx,
            client_capabilities: capabilities,
        })
    }

    pub async fn initialize(
        &mut self,
        client_info: Implementation,
    ) -> Result<InitializeResult, ModelContextProtocolClientError> {
        let params = InitializeRequestParams {
            protocol_version: "2024-11-05".to_string(),
            capabilities: self.client_capabilities.clone(),
            client_info,
        };
        let response = self.request("initialize".to_string(), serde_json::to_value(params)?).await?;
        let result: InitializeResult = serde_json::from_value(response)?;

        // Store server capabilities
        let mut server_capabilities = self.server_capabilities.write().await;
        *server_capabilities = Some(result.capabilities.clone());

        Ok(result)
    }

    pub async fn initialized(&mut self) -> Result<(), ModelContextProtocolClientError> {
        let params = InitializedNotificationParams { meta: None };
        self.notify("notifications/initialized".to_string(), serde_json::to_value(params)?).await?;
        Ok(())
    }

    pub async fn list_resources(
        &mut self,
        params: Option<ListResourcesRequestParams>,
    ) -> Result<ListResourcesResult, ModelContextProtocolClientError> {
        debug!("Server {} - Sending resources/list request", self.server.name);
        let response = self.request("resources/list".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn read_resource(
        &mut self,
        params: ReadResourceRequestParams,
    ) -> Result<ReadResourceResult, ModelContextProtocolClientError> {
        debug!("Server {} - Sending resources/read request", self.server.name);
        let response = self.request("resources/read".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn list_prompts(
        &mut self,
        params: Option<ListPromptsRequestParams>,
    ) -> Result<ListPromptsResult, ModelContextProtocolClientError> {
        debug!("Server {} - Sending prompts/list request", self.server.name);
        let response = self.request("prompts/list".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn get_prompt(
        &mut self,
        params: GetPromptRequestParams,
    ) -> Result<GetPromptResult, ModelContextProtocolClientError> {
        debug!("Server {} - Sending prompts/get request", self.server.name);
        let response = self.request("prompts/get".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn list_tools(
        &mut self,
        params: Option<ListToolsRequestParams>,
    ) -> Result<ListToolsResult, ModelContextProtocolClientError> {
        debug!("Server {} - Sending tools/list request", self.server.name);
        let response = self.request("tools/list".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn call_tool(
        &mut self,
        params: CallToolRequestParams,
    ) -> Result<CallToolResult, ModelContextProtocolClientError> {
        debug!("Server {} - Sending tools/call request", self.server.name);
        let response = self.request("tools/call".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn request(
        &mut self,
        method: String,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ModelContextProtocolClientError> {
        let mut counter = self.request_counter.write().await;
        *counter += 1;
        let id = *counter;

        // Create proper JSON-RPC 2.0 request
        let request = jsonrpc_core::MethodCall {
            jsonrpc: Some(jsonrpc_core::Version::V2),
            method,
            params: Params::Map(params.as_object().cloned().unwrap_or_default()),
            id: jsonrpc_core::Id::Num(id),
        };

        // Create response channel
        let (response_tx, response_rx) = oneshot::channel();

        // Register pending request
        {
            let mut pending = self.pending_requests.lock().await;
            pending.insert(id, response_tx);
        }

        // Send request
        if let Err(e) = self.transport.send(request.into(), serde_json::Value::Null).await {
            // Clean up pending request on send error
            let mut pending = self.pending_requests.lock().await;
            pending.remove(&id);
            return Err(ModelContextProtocolClientError::Transport(format!("Send: {}", e).into()));
        }

        // Wait for response with timeout
        match tokio::time::timeout(std::time::Duration::from_secs(self.server.request_timeout), response_rx).await {
            Ok(response) => match response {
                Ok(result) => result,
                Err(_) => Err(ModelContextProtocolClientError::Request("Response channel closed".into())),
            },
            Err(_) => {
                // Clean up pending request on timeout
                let mut pending = self.pending_requests.lock().await;
                pending.remove(&id);
                Err(ModelContextProtocolClientError::Request("Request timed out".into()))
            }
        }
    }

    pub async fn notify(
        &mut self,
        method: String,
        params: serde_json::Value,
    ) -> Result<(), ModelContextProtocolClientError> {
        // Create JSON-RPC 2.0 notification
        let notification = jsonrpc_core::Notification {
            jsonrpc: Some(jsonrpc_core::Version::V2),
            method,
            params: Params::Map(params.as_object().cloned().unwrap_or_default()),
        };

        // Send notification without waiting for response
        self.transport
            .send(notification.into(), serde_json::Value::Null)
            .await
            .map_err(|e| ModelContextProtocolClientError::Transport(format!("Send: {}", e).into()))
    }

    pub async fn close(&mut self) -> Result<(), ModelContextProtocolClientError> {
        self.transport
            .close()
            .await
            .map_err(|e| ModelContextProtocolClientError::Transport(format!("Close: {}", e).into()))?;
        self.start_handle.abort();
        Ok(())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ModelContextProtocolClientError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Invalid transport type: {0}")]
    Transport(Cow<'static, str>),
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("Request: {0}")]
    Request(Cow<'static, str>),
    #[error("Response ID mismatch: expected {expected}, got {actual:?}")]
    ResponseIdMismatch { expected: u64, actual: jsonrpc_core::Id },
    #[error("MCP Client not initialized")]
    ClientNotInitialized,
    #[error("Configuration error: {0}")]
    Configuration(Cow<'static, str>),
}

impl ActorError for ModelContextProtocolClientError {}

// Remove Debug derive and implement it manually
#[derive(Serialize, Deserialize)]
pub struct ModelContextProtocolClientActor {
    server: ServerConfig,
    tools: Option<ListToolsResult>,
    client_capabilities: ClientCapabilities,
    #[serde(skip)]
    client: Option<Arc<Mutex<ModelContextProtocolClient>>>,
    #[serde(skip)]
    sampling_handler: Option<Arc<dyn SamplingHandler + Send + Sync>>,
    server_capabilities: Option<ServerCapabilities>,
}

// Manual implementation of Debug for ModelContextProtocolClientActor
impl std::fmt::Debug for ModelContextProtocolClientActor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelContextProtocolClientActor")
            .field("server", &self.server)
            .field("tools", &self.tools)
            .field("client_capabilities", &self.client_capabilities)
            .field("server_capabilities", &self.server_capabilities)
            // Skip client and sampling_handler fields as they don't implement Debug
            .finish_non_exhaustive()
    }
}

impl ModelContextProtocolClientActor {
    pub fn new(server: ServerConfig, client_capabilities: ClientCapabilities) -> Self {
        ModelContextProtocolClientActor {
            server,
            tools: None,
            client: None,
            client_capabilities,
            sampling_handler: None,
            server_capabilities: None,
        }
    }

    pub fn with_sampling_handler(mut self, handler: Arc<dyn SamplingHandler + Send + Sync>) -> Self {
        // Automatically enable sampling capability if setting a handler
        if self.client_capabilities.sampling.is_none() {
            self.client_capabilities.sampling = Some(BTreeMap::new());
        }
        self.sampling_handler = Some(handler);
        self
    }
}

impl Actor for ModelContextProtocolClientActor {
    type Error = ModelContextProtocolClientError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), ModelContextProtocolClientError> {
        info!("{} Started (server: {})", ctx.id(), self.server.name);

        let mut client = ModelContextProtocolClient::new(
            self.server.clone(),
            self.client_capabilities.clone(),
            self.sampling_handler.clone(),
        )
        .await?;

        // Initialize the client
        let init_result =
            client.initialize(Implementation { name: self.server.name.clone(), version: "0.1.0".to_string() }).await?;
        info!("Server {} capabilities: {:?}", self.server.name, init_result.capabilities);
        self.server_capabilities = Some(init_result.capabilities);

        // Notify the server that the client has initialized
        client.initialized().await?;

        self.client = Some(Arc::new(Mutex::new(client)));

        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(input) = frame.is::<CallTool>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {} {:?}", ctx.id(), self.server.name, err);
                }
            } else if let Some(input) = frame.is::<ListTools>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {} {:?}", ctx.id(), self.server.name, err);
                }
            }
        }

        info!("{} Finished (server: {})", ctx.id(), self.server.name);
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTools(pub Option<ListToolsRequestParams>);

impl Message<ListTools> for ModelContextProtocolClientActor {
    type Response = ListToolsResult;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: &ListTools,
    ) -> Result<(), ModelContextProtocolClientError> {
        let Some(client) = &self.client else { return Err(ModelContextProtocolClientError::ClientNotInitialized) };
        let mut client = client.lock().await;
        let response = client.list_tools(message.0.clone()).await?;
        self.tools = Some(response.clone());
        self.save(ctx).await?;
        ctx.reply(response).await?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallTool(pub CallToolRequestParams);

impl Message<CallTool> for ModelContextProtocolClientActor {
    type Response = CallToolResult;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: &CallTool,
    ) -> Result<(), ModelContextProtocolClientError> {
        let Some(client) = &self.client else { return Err(ModelContextProtocolClientError::ClientNotInitialized) };
        let mut client = client.lock().await;
        let response = client.call_tool(message.0.clone()).await?;
        ctx.reply(response).await?;
        Ok(())
    }
}
