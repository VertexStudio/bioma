use crate::schema::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, GetPromptRequestParams, GetPromptResult, Implementation,
    InitializeRequestParams, InitializeResult, InitializedNotificationParams, ListPromptsRequestParams,
    ListPromptsResult, ListResourcesRequestParams, ListResourcesResult, ListToolsRequestParams, ListToolsResult,
    ReadResourceRequestParams, ReadResourceResult, ServerCapabilities,
};
use crate::transport::{stdio::StdioTransport, Transport, TransportType};
use crate::JsonRpcMessage;
use anyhow::Error;
use bioma_actor::prelude::*;
use jsonrpc_core::Params;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StdioConfig {
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SseConfig {
    pub endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "transport")]
pub enum TransportConfig {
    #[serde(rename = "stdio")]
    Stdio(StdioConfig),
    #[serde(rename = "sse")]
    Sse(SseConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    pub transport: TransportConfig,
    #[serde(default = "default_request_timeout")]
    pub request_timeout: u64,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    start_handle: JoinHandle<Result<(), Error>>,
    on_message_rx: mpsc::Receiver<JsonRpcMessage>,
    #[allow(unused)]
    on_error_rx: mpsc::Receiver<Error>,
    #[allow(unused)]
    on_close_rx: mpsc::Receiver<()>,
}

impl ModelContextProtocolClient {
    pub async fn new(server: ServerConfig) -> Result<Self, ModelContextProtocolClientError> {
        let (on_message_tx, on_message_rx) = mpsc::channel::<JsonRpcMessage>(1);
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
            TransportConfig::Sse(_config) => {
                unimplemented!("SSE transport not implemented");
            }
        };

        // Start transport once during initialization
        let start_handle = transport
            .start()
            .await
            .map_err(|e| ModelContextProtocolClientError::Transport(format!("Start: {}", e).into()))?;

        Ok(Self {
            server,
            transport,
            server_capabilities: Arc::new(RwLock::new(None)),
            request_counter: Arc::new(RwLock::new(0)),
            start_handle,
            on_message_rx,
            on_error_rx,
            on_close_rx,
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
        let mut counter: tokio::sync::RwLockWriteGuard<'_, u64> = self.request_counter.write().await;
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
        if let Err(e) = self.transport.send(request.into()).await {
            return Err(ModelContextProtocolClientError::Transport(format!("Send: {}", e).into()));
        }

        // Wait for response with timeout
        match tokio::time::timeout(
            std::time::Duration::from_secs(self.server.request_timeout),
            self.on_message_rx.recv(),
        )
        .await
        {
            Ok(Some(response)) => {
                match response {
                    JsonRpcMessage::Response(jsonrpc_core::Response::Single(output)) => match output {
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
        // Create JSON-RPC 2.0 notification
        let notification = jsonrpc_core::Notification {
            jsonrpc: Some(jsonrpc_core::Version::V2),
            method,
            params: Params::Map(params.as_object().cloned().unwrap_or_default()),
        };

        // Send notification without waiting for response
        self.transport
            .send(notification.into())
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
