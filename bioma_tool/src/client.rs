use crate::schema::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, GetPromptRequestParams, GetPromptResult, Implementation,
    InitializeRequestParams, InitializeResult, ListPromptsRequestParams, ListPromptsResult, ListResourcesRequestParams,
    ListResourcesResult, ListToolsRequestParams, ListToolsResult, ReadResourceRequestParams, ReadResourceResult,
    ServerCapabilities,
};
use crate::transport::{stdio::StdioTransport, Transport, TransportType};
use bioma_actor::prelude::*;
use jsonrpc_core::Params;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::{interval, Duration};
use tracing::{debug, error, info};

const DEFAULT_PING_INTERVAL_SECS: u64 = 30;
const DEFAULT_PING_TIMEOUT_SECS: u64 = 5;
const MAX_CONSECUTIVE_FAILURES: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub name: String,
    #[serde(default = "default_transport")]
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingConfig {
    #[serde(default = "default_ping_interval")]
    pub interval_secs: u64,
    #[serde(default = "default_ping_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_failures")]
    pub max_failures: u32,
}

impl Default for PingConfig {
    fn default() -> Self {
        Self {
            interval_secs: default_ping_interval(),
            timeout_secs: default_ping_timeout(),
            max_failures: default_max_failures(),
        }
    }
}

fn default_ping_interval() -> u64 {
    DEFAULT_PING_INTERVAL_SECS
}

fn default_ping_timeout() -> u64 {
    DEFAULT_PING_TIMEOUT_SECS
}

fn default_max_failures() -> u32 {
    MAX_CONSECUTIVE_FAILURES
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub host: bool,
    pub server: ServerConfig,
}

fn default_transport() -> String {
    "stdio".to_string()
}

#[derive(Clone)]
pub struct ModelContextProtocolClient {
    transport: TransportType,
    pub server_capabilities: Arc<RwLock<Option<ServerCapabilities>>>,
    request_counter: Arc<RwLock<u64>>,
    response_rx: Arc<Mutex<mpsc::Receiver<String>>>,
    ping_failures: Arc<RwLock<u32>>,
}

impl ModelContextProtocolClient {
    pub async fn new(
        server: ServerConfig,
        ping_config: Option<PingConfig>,
    ) -> Result<Self, ModelContextProtocolClientError> {
        println!("Server config: {:?}", server);
        let (tx, rx) = mpsc::channel::<String>(1);

        let transport = StdioTransport::new_client(&server);
        let transport = match transport {
            Ok(transport) => transport,
            Err(e) => {
                return Err(ModelContextProtocolClientError::Transport(format!("Client new: {}", e.to_string()).into()))
            }
        };

        let transport = match server.transport.as_str() {
            "stdio" => TransportType::Stdio(transport),
            _ => {
                return Err(ModelContextProtocolClientError::Transport(
                    format!("Invalid transport type: {}", server.transport.as_str()).into(),
                ))
            }
        };

        // Start transport once during initialization
        let mut transport_clone = transport.clone();
        tokio::spawn(async move {
            if let Err(e) = transport_clone.start(tx).await {
                error!("Transport error: {}", e);
            }
        });

        // Create client first
        let client = Self {
            transport,
            server_capabilities: Arc::new(RwLock::new(None)),
            request_counter: Arc::new(RwLock::new(0)),
            response_rx: Arc::new(Mutex::new(rx)),
            ping_failures: Arc::new(RwLock::new(0)),
        };

        // Start ping task with configurable interval if ping config is provided
        if let Some(ping_config) = ping_config {
            let client_arc = Arc::new(Mutex::new(client.clone()));
            Self::start_ping_task(client_arc, ping_config).await;
        }

        // Return the original client
        Ok(client)
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

    pub async fn list_resources(
        &mut self,
        params: Option<ListResourcesRequestParams>,
    ) -> Result<ListResourcesResult, ModelContextProtocolClientError> {
        debug!("Sending resources/list request");
        let response = self.request("resources/list".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn read_resource(
        &mut self,
        params: ReadResourceRequestParams,
    ) -> Result<ReadResourceResult, ModelContextProtocolClientError> {
        debug!("Sending resources/read request");
        let response = self.request("resources/read".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn list_prompts(
        &mut self,
        params: Option<ListPromptsRequestParams>,
    ) -> Result<ListPromptsResult, ModelContextProtocolClientError> {
        debug!("Sending prompts/list request");
        let response = self.request("prompts/list".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn get_prompt(
        &mut self,
        params: GetPromptRequestParams,
    ) -> Result<GetPromptResult, ModelContextProtocolClientError> {
        debug!("Sending prompts/get request");
        let response = self.request("prompts/get".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn list_tools(
        &mut self,
        params: Option<ListToolsRequestParams>,
    ) -> Result<ListToolsResult, ModelContextProtocolClientError> {
        debug!("Sending tools/list request");
        let response = self.request("tools/list".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn call_tool(
        &mut self,
        params: CallToolRequestParams,
    ) -> Result<CallToolResult, ModelContextProtocolClientError> {
        debug!("Sending tools/call request");
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
            params: Params::Map(params.as_object().map(|obj| obj.clone()).unwrap_or_default()),
            id: jsonrpc_core::Id::Num(id),
        };

        // Send request
        let request_str = serde_json::to_string(&request)?;
        if let Err(e) = self.transport.send(request_str).await {
            return Err(ModelContextProtocolClientError::Transport(format!("Send: {}", e.to_string()).into()));
        }

        // Parse response as proper JSON-RPC response
        if let Some(response) = self.response_rx.lock().await.recv().await {
            let response: jsonrpc_core::Response = serde_json::from_str(&response)?;
            match response {
                jsonrpc_core::Response::Single(output) => match output {
                    jsonrpc_core::Output::Success(success) => Ok(success.result),
                    jsonrpc_core::Output::Failure(failure) => {
                        error!("RPC error: {:?}", failure.error);
                        Err(ModelContextProtocolClientError::Request(format!("RPC error: {:?}", failure.error).into()))
                    }
                },
                _ => Err(ModelContextProtocolClientError::Request("Unexpected response type".into())),
            }
        } else {
            Err(ModelContextProtocolClientError::Request("No response received".into()))
        }
    }

    async fn ping(&mut self) -> Result<(), ModelContextProtocolClientError> {
        debug!("Sending ping request");

        let ping_future = self.request("ping".to_string(), serde_json::Value::Object(serde_json::Map::new()));

        match tokio::time::timeout(Duration::from_secs(DEFAULT_PING_TIMEOUT_SECS), ping_future).await {
            Ok(result) => match result {
                Ok(response) => {
                    if response != serde_json::Value::Object(serde_json::Map::new()) {
                        self.increment_ping_failures().await;
                        return Err(ModelContextProtocolClientError::Request("Invalid ping response".into()));
                    }
                    // Reset failures on successful ping
                    *self.ping_failures.write().await = 0;
                    Ok(())
                }
                Err(e) => {
                    self.increment_ping_failures().await;
                    Err(e)
                }
            },
            Err(_) => {
                self.increment_ping_failures().await;
                Err(ModelContextProtocolClientError::Request("Ping timeout".into()))
            }
        }
    }

    async fn increment_ping_failures(&self) {
        let mut failures = self.ping_failures.write().await;
        *failures += 1;
        error!("Ping failure count: {}", *failures);
    }

    async fn start_ping_task(client: Arc<Mutex<Self>>, config: PingConfig) {
        let mut interval = interval(Duration::from_secs(config.interval_secs));

        tokio::spawn(async move {
            loop {
                interval.tick().await;
                let mut client = client.lock().await;

                if let Err(e) = client.ping().await {
                    error!("Ping failed: {}", e);

                    // Use the configured max_failures instead of constant
                    let failures = *client.ping_failures.read().await;
                    if failures >= config.max_failures {
                        error!(
                            "Maximum consecutive ping failures ({}) reached. Connection may be dead.",
                            config.max_failures
                        );
                        *client.ping_failures.write().await = 0; // Reset counter
                    }
                }
            }
        });
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
    #[error("MCP Client not initialized")]
    ClientNotInitialized,
}

impl ActorError for ModelContextProtocolClientError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelContextProtocolClientActor {
    server: ServerConfig,
    ping_config: Option<PingConfig>,
    #[serde(skip)]
    client: Option<Arc<Mutex<ModelContextProtocolClient>>>,
}

impl ModelContextProtocolClientActor {
    pub fn new(server: ServerConfig, ping_config: Option<PingConfig>) -> Self {
        ModelContextProtocolClientActor { server, ping_config, client: None }
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
        info!("{} Started", ctx.id());

        let client = ModelContextProtocolClient::new(self.server.clone(), self.ping_config.clone()).await?;
        self.client = Some(Arc::new(Mutex::new(client)));

        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(input) = frame.is::<CallTool>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            } else if let Some(input) = frame.is::<ListTools>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            }
        }

        info!("{} Finished", ctx.id());
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
