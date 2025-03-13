use crate::schema::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, GetPromptRequestParams, GetPromptResult, Implementation,
    InitializeRequestParams, InitializeResult, InitializedNotificationParams, ListPromptsRequestParams,
    ListPromptsResult, ListResourcesRequestParams, ListResourcesResult, ListToolsRequestParams, ListToolsResult,
    ReadResourceRequestParams, ReadResourceResult, ServerCapabilities,
};
use crate::transport::sse::{SseClientConfig, SseTransport};
use crate::transport::stdio::StdioServerConfig;
use crate::transport::{stdio::StdioTransport, Transport, TransportType};
use bioma_actor::prelude::*;
use jsonrpc_core::Params;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{debug, error, info};

#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct ServerConfig {
    pub name: String,
    #[builder(default = default_version())]
    pub version: String,
    #[builder(default = default_request_timeout())]
    pub request_timeout: u64,
    #[serde(flatten)]
    pub transport: TransportConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransportConfig {
    Stdio(StdioServerConfig),
    Sse(SseClientConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct ClientConfig {
    pub server: ServerConfig,
}

fn default_version() -> String {
    "0.1.0".to_string()
}

fn default_request_timeout() -> u64 {
    5
}

pub struct ModelContextProtocolClient {
    server: ServerConfig,
    transport: TransportType,
    pub server_capabilities: Arc<RwLock<Option<ServerCapabilities>>>,
    request_counter: Arc<RwLock<u64>>,
    response_rx: mpsc::Receiver<String>,
}

impl ModelContextProtocolClient {
    pub async fn new(server: ServerConfig) -> Result<Self, ModelContextProtocolClientError> {
        let (tx, rx) = mpsc::channel::<String>(1);

        // Create the appropriate transport based on configuration
        let transport = match &server.transport {
            TransportConfig::Stdio(config) => TransportType::Stdio(
                StdioTransport::new_client(config)
                    .map_err(|e| ModelContextProtocolClientError::Transport(format!("Stdio init: {}", e).into()))?,
            ),
            TransportConfig::Sse(config) => TransportType::Sse(
                SseTransport::new_client(config.clone())
                    .map_err(|e| ModelContextProtocolClientError::Transport(format!("SSE init: {}", e).into()))?,
            ),
        };

        // Start transport once during initialization
        let mut transport_clone = transport.clone();
        tokio::spawn(async move {
            if let Err(e) = transport_clone.start(tx).await {
                error!("Transport error: {}", e);
            }
        });

        Ok(Self {
            server,
            transport,
            server_capabilities: Arc::new(RwLock::new(None)),
            request_counter: Arc::new(RwLock::new(0)),
            response_rx: rx,
        })
    }

    pub async fn initialize(
        &mut self,
        client_info: Implementation,
    ) -> Result<InitializeResult, ModelContextProtocolClientError> {
        let params = InitializeRequestParams {
            protocol_version: "2024-11-05".to_string(),
            capabilities: ClientCapabilities::default(),
            client_info,
        };
        let response = self.request("initialize".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
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

        // Send request
        let request_str = serde_json::to_string(&request)?;
        if let Err(e) = self.transport.send(request_str).await {
            return Err(ModelContextProtocolClientError::Transport(format!("Send: {}", e).into()));
        }

        // Wait for response with timeout
        match tokio::time::timeout(std::time::Duration::from_secs(self.server.request_timeout), self.response_rx.recv())
            .await
        {
            Ok(Some(response)) => {
                let response: jsonrpc_core::Response = serde_json::from_str(&response)?;
                match response {
                    jsonrpc_core::Response::Single(output) => match output {
                        jsonrpc_core::Output::Success(success) => {
                            // Verify that response ID matches request ID
                            if success.id != jsonrpc_core::Id::Num(id) {
                                return Err(ModelContextProtocolClientError::ResponseIdMismatch {
                                    expected: id,
                                    actual: success.id,
                                });
                            }
                            Ok(success.result)
                        }
                        jsonrpc_core::Output::Failure(failure) => {
                            error!("RPC error: {:?}", failure.error);
                            Err(ModelContextProtocolClientError::Request(
                                format!("RPC error: {:?}", failure.error).into(),
                            ))
                        }
                    },
                    _ => Err(ModelContextProtocolClientError::Request("Unexpected response type".into())),
                }
            }
            Ok(None) => Err(ModelContextProtocolClientError::Request("No response received".into())),
            Err(_) => Err(ModelContextProtocolClientError::Request("Request timed out".into())),
        }
    }

    pub async fn notify(
        &mut self,
        method: String,
        params: serde_json::Value,
    ) -> Result<(), ModelContextProtocolClientError> {
        // Create JSON-RPC 2.0 notification (no id)
        let notification = jsonrpc_core::Notification {
            jsonrpc: Some(jsonrpc_core::Version::V2),
            method,
            params: Params::Map(params.as_object().cloned().unwrap_or_default()),
        };

        // Send notification without waiting for response
        let request_str = serde_json::to_string(&notification)?;
        self.transport
            .send(request_str)
            .await
            .map_err(|e| ModelContextProtocolClientError::Transport(format!("Send: {}", e).into()))
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
}

impl ActorError for ModelContextProtocolClientError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelContextProtocolClientActor {
    server: ServerConfig,
    tools: Option<ListToolsResult>,
    #[serde(skip)]
    client: Option<Arc<Mutex<ModelContextProtocolClient>>>,
    server_capabilities: Option<ServerCapabilities>,
}

impl ModelContextProtocolClientActor {
    pub fn new(server: ServerConfig) -> Self {
        ModelContextProtocolClientActor { server, tools: None, client: None, server_capabilities: None }
    }
}

impl std::fmt::Debug for ModelContextProtocolClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ModelContextProtocolClient")
    }
}

impl Actor for ModelContextProtocolClientActor {
    type Error = ModelContextProtocolClientError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), ModelContextProtocolClientError> {
        info!("{} Started (server: {})", ctx.id(), self.server.name);

        let mut client = ModelContextProtocolClient::new(self.server.clone()).await?;

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
