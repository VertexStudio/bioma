use anyhow::Result;
use bioma_actor::prelude::*;
use bioma_tool::client::{
    CallTool, ClientConfig, ListTools, ModelContextProtocolClientActor, ModelContextProtocolClientError, ServerConfig,
};
use bioma_tool::schema::{CallToolRequestParams, CallToolResult, ListToolsResult};
use ollama_rs::generation::tools::{ToolCall, ToolInfo};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
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
    pub server: ServerConfig,
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
        info!("Tools from {} ({})", self.server.name, list_tools.tools.len());
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
    pub async fn new(engine: &Engine, configs: Vec<ClientConfig>, base_id: String) -> Result<Self, ToolsHubError> {
        let mut hub = Self { clients: vec![] };

        for config in configs {
            let server = config.server;
            let client_id = ActorId::of::<ModelContextProtocolClientActor>(format!("{}/{}", base_id, server.name));

            let client_handle = {
                debug!("Spawning ModelContextProtocolClient actor for client {}", client_id);
                let (mut client_ctx, mut client_actor) = Actor::spawn(
                    engine.clone(),
                    client_id.clone(),
                    ModelContextProtocolClientActor::new(server.clone()),
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

            hub.clients.push(ToolClient { server, client_id, _client_handle: client_handle, tools: vec![] });
        }

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
