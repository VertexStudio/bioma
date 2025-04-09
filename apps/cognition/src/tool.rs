use anyhow::Result;
use bioma_actor::prelude::*;
use bioma_mcp::client::{Client, ClientConfig, ClientError, ModelContextProtocolClient, ServerConfig};
use bioma_mcp::progress::Progress;
use bioma_mcp::schema::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, CreateMessageRequestParams, CreateMessageResult,
    Implementation, ListToolsRequestParams, ListToolsResult, Root,
};
use ollama_rs::generation::tools::{ToolCall, ToolInfo};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

// Custom error type for our ToolsHub actor.
#[derive(thiserror::Error, Debug)]
pub enum ToolsHubError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Client error: {0}")]
    Client(#[from] ModelContextProtocolClientError),
}
impl ActorError for ToolsHubError {}

// -----------------------------------------------------------------------------
// ToolClient: represents a tool-hosting client with methods that use the actor context
// -----------------------------------------------------------------------------
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolClient {
    pub servers: Vec<ServerConfig>,
    pub client_id: ActorId,
    #[serde(skip)]
    pub _client_handle: Option<JoinHandle<()>>,
    pub tools: Vec<ToolInfo>,
}

impl ToolClient {
    pub async fn call<T: Actor>(
        &self,
        ctx: &ActorContext<T>,
        tool_call: &ToolCall,
    ) -> Result<CallToolResult, ToolsHubError> {
        let args: BTreeMap<String, Value> = tool_call
            .function
            .arguments
            .as_object()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        let request = CallToolRequestParams { name: tool_call.function.name.clone(), arguments: Some(args) };
        let response = ctx
            .send_and_wait_reply::<ModelContextProtocolClientActor, CallTool>(
                CallTool(request),
                &self.client_id,
                SendOptions::default(),
            )
            .await?;
        Ok(response)
    }

    pub async fn list_tools<T: Actor>(
        &self,
        ctx: &ActorContext<T>,
        tools_actor: &ActorId,
    ) -> Result<Vec<ToolInfo>, ToolsHubError> {
        let list_tools: ListToolsResult = ctx
            .send_and_wait_reply::<ModelContextProtocolClientActor, ListTools>(
                ListTools(None),
                tools_actor,
                SendOptions::default(),
            )
            .await?;
        info!("Fetched tools:");
        for tool in &list_tools.tools {
            info!("├─ {}", tool.name);
        }
        let tools: Vec<ToolInfo> = list_tools
            .tools
            .into_iter()
            .map(|tool| {
                ToolInfo::from_schema(
                    tool.name.into(),
                    tool.description.unwrap_or_default().into(),
                    tool.input_schema.clone(),
                )
            })
            .collect();
        Ok(tools)
    }

    pub async fn health<T: Actor>(&self, ctx: &ActorContext<T>) -> Result<bool, ToolsHubError> {
        let health = ctx.check_actor_health(&self.client_id).await?;
        Ok(health)
    }
}

// -----------------------------------------------------------------------------
// ToolsHub: the actor that manages multiple ToolClients and handles ListTools/CallTool messages
// -----------------------------------------------------------------------------
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolsHub {
    pub clients: Vec<ToolClient>,
}

impl ToolsHub {
    pub async fn new(engine: &Engine, config: ClientConfig, base_id: String) -> Result<Self, ToolsHubError> {
        let mut hub = Self { clients: vec![] };

        let client_id = ActorId::of::<ModelContextProtocolClientActor>(format!("{}/{}", base_id, config.name));

        let client_handle = {
            debug!("Spawning ModelContextProtocolClient actor for client {}", client_id);
            let (mut client_ctx, mut client_actor) = Actor::spawn(
                engine.clone(),
                client_id.clone(),
                ModelContextProtocolClientActor::new(config.servers.clone()),
                SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
            )
            .await?;

            let client_id_spawn = client_id.clone();
            Some(tokio::spawn(async move {
                if let Err(e) = client_actor.start(&mut client_ctx).await {
                    error!("ModelContextProtocolClient actor error: {} for client {}", e, client_id_spawn);
                }
            }))
        };

        hub.clients.push(ToolClient {
            servers: config.servers,
            client_id,
            _client_handle: client_handle,
            tools: vec![],
        });

        Ok(hub)
    }

    /// Returns the first tool that matches the given name.
    pub fn get_tool(&self, tool_name: &str) -> Option<(ToolInfo, &ToolClient)> {
        for client in &self.clients {
            for tool in &client.tools {
                if tool.name() == tool_name {
                    return Some((tool.clone(), client));
                }
            }
        }
        None
    }

    /// Refreshes the tools from all clients.
    pub async fn refresh_tools<T: Actor>(
        &mut self,
        ctx: &ActorContext<T>,
        tools_actor: &ActorId,
    ) -> Result<Vec<ToolInfo>, ToolsHubError> {
        let mut all_tools = Vec::new();
        for client in &mut self.clients {
            match client.list_tools(ctx, tools_actor).await {
                Ok(tools) => {
                    client.tools = tools.clone();
                    all_tools.extend(tools);
                }
                Err(e) => error!("Failed to fetch tools: {}", e),
            }
        }
        Ok(all_tools)
    }

    /// Processes a received message.
    async fn process_message(
        &mut self,
        ctx: &mut ActorContext<Self>,
        frame: &FrameMessage,
    ) -> Result<(), ToolsHubError> {
        if let Some(input) = frame.is::<ListTools>() {
            self.reply(ctx, &input, frame).await?;
        } else if let Some(input) = frame.is::<ToolCall>() {
            self.reply(ctx, &input, frame).await?;
        }
        Ok(())
    }
}

impl Actor for ToolsHub {
    type Error = ToolsHubError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("ToolsHub actor started: {}", ctx.id());

        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Err(err) = self.process_message(ctx, &frame).await {
                error!("{} {:?}", ctx.id(), err);
            }
        }
        info!("{} Finished", ctx.id());
        Ok(())
    }
}

impl Message<ListTools> for ToolsHub {
    type Response = Vec<ToolInfo>;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, _message: &ListTools) -> Result<(), ToolsHubError> {
        let mut all_tools = Vec::new();
        for client in &mut self.clients {
            if client.tools.is_empty() {
                match client.list_tools(ctx, &client.client_id).await {
                    Ok(tools) => {
                        client.tools = tools.clone();
                        all_tools.extend(tools);
                    }
                    Err(e) => {
                        error!("Failed to fetch tools: {}", e);
                        continue;
                    }
                }
            } else {
                all_tools.extend(client.tools.clone());
            }
        }
        ctx.reply(all_tools).await?;
        Ok(())
    }
}

impl Message<ToolCall> for ToolsHub {
    type Response = CallToolResult;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, message: &ToolCall) -> Result<(), ToolsHubError> {
        if let Some((_tool_info, client)) = self.get_tool(&message.function.name) {
            let result = client.call(ctx, message).await?;
            ctx.reply(result).await?;
        } else {
            ctx.reply(CallToolResult {
                meta: None,
                content: vec![serde_json::json!({
                    "error": format!("Tool {} not found", message.function.name)
                })],
                is_error: Some(true),
            })
            .await?;
        }
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
    #[error("MCP Client not initialized")]
    ClientNotInitialized,
    #[error("Client error: {0}")]
    Client(#[from] ClientError),
    #[error("Error: {0}")]
    Custom(#[from] anyhow::Error),
}

impl ActorError for ModelContextProtocolClientError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelContextProtocolClientActor {
    servers: Vec<ServerConfig>,
    #[serde(skip)]
    client: Option<Arc<Mutex<Client<McpBasicClient>>>>,
    tools: Option<ListToolsResult>,
}

impl ModelContextProtocolClientActor {
    pub fn new(servers: Vec<ServerConfig>) -> Self {
        ModelContextProtocolClientActor { servers, client: None, tools: None }
    }
}

#[derive(Clone)]
pub struct McpBasicClient {
    servers: Vec<ServerConfig>,
}

impl ModelContextProtocolClient for McpBasicClient {
    async fn get_server_configs(&self) -> Vec<ServerConfig> {
        self.servers.clone()
    }

    async fn get_capabilities(&self) -> ClientCapabilities {
        ClientCapabilities::default()
    }

    async fn get_roots(&self) -> Vec<Root> {
        vec![]
    }

    async fn on_create_message(
        &self,
        _params: CreateMessageRequestParams,
        _progress: Progress,
    ) -> Result<CreateMessageResult, ClientError> {
        todo!()
    }
}

impl Actor for ModelContextProtocolClientActor {
    type Error = ModelContextProtocolClientError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), ModelContextProtocolClientError> {
        info!("{} Started", ctx.id());
        for server in &self.servers {
            info!("└─ Server: {}", server.name);
        }

        let mut client = Client::new(McpBasicClient { servers: self.servers.clone() }).await?;

        // Initialize the client
        let init_result = client
            .initialize(Implementation { name: "tools-client".to_string(), version: "0.1.0".to_string() })
            .await?;

        for (server, result) in init_result {
            info!("Server '{}' capabilities: {:#?}", server, result.capabilities);
        }

        // Notify the server that the client has initialized
        client.initialized().await?;

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

        info!("{} Finished (servers: {})", ctx.id(), self.servers.len());
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
        let response = client.list_tools(message.0.clone()).await?.await?;
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
        let response = client.call_tool(message.0.clone(), false).await?.await?;
        ctx.reply(response).await?;
        Ok(())
    }
}
