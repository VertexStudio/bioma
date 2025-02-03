use anyhow::Result;
use async_trait::async_trait;
use bioma_actor::prelude::*;
use bioma_tool::client::{
    CallTool, ClientConfig, ListTools, ModelContextProtocolClientActor, ModelContextProtocolClientError, ServerConfig,
};
use bioma_tool::schema::{self, CallToolRequestParams, CallToolResult, ListToolsResult, ToolInputSchema};
use ollama_rs::generation::tools::{ToolCall, ToolInfo};
use schemars::{
    schema::{
        ArrayValidation, InstanceType, Metadata, ObjectValidation, RootSchema, Schema, SchemaObject, SingleOrVec,
    },
    Map,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

// Custom error type for our ToolsHub actor.
#[derive(thiserror::Error, Debug)]
pub enum ToolsHubError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Error: {0}")]
    Any(#[from] anyhow::Error),
    #[error("Client error: {0}")]
    Client(#[from] ModelContextProtocolClientError),
}
impl ActorError for ToolsHubError {}

// -----------------------------------------------------------------------------
// ToolClient: represents a tool-hosting client with methods that use the actor context
// -----------------------------------------------------------------------------
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolClient {
    pub hosting: bool,
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
                SendOptions::builder().timeout(Duration::from_secs(30)).check_health(true).build(),
            )
            .await?;
        Ok(response)
    }

    pub async fn list_tools<T: Actor>(&self, ctx: &ActorContext<T>) -> Result<Vec<ToolInfo>, ToolsHubError> {
        let list_tools: ListToolsResult = ctx
            .send_and_wait_reply::<ModelContextProtocolClientActor, ListTools>(
                ListTools(None),
                &self.client_id,
                SendOptions::builder().timeout(Duration::from_secs(30)).check_health(true).build(),
            )
            .await?;
        info!("Tools from {} ({})", self.server.name, list_tools.tools.len());
        for tool in &list_tools.tools {
            info!("├─ {}", tool.name);
        }
        let tools: Vec<ToolInfo> = list_tools.tools.into_iter().map(|tool| ToolsHub::parse_tool_info(tool)).collect();
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
    pub fn new() -> Self {
        Self { clients: vec![] }
    }

    pub async fn add_tool(
        &mut self,
        engine: &Engine,
        config: ClientConfig,
        prefix: String,
    ) -> Result<(), ToolsHubError> {
        let hosting = config.host;
        let server = config.server;
        let client_id = ActorId::of::<ModelContextProtocolClientActor>(format!("{}/{}", prefix, server.name));
        // If hosting, spawn the ModelContextProtocolClient actor.
        let client_handle = if config.host {
            debug!("Spawning ModelContextProtocolClient actor for client {}", client_id);
            let (mut client_ctx, mut client_actor) = Actor::spawn(
                engine.clone(),
                client_id.clone(),
                ModelContextProtocolClientActor::new(server.clone()),
                SpawnOptions::builder()
                    .exists(SpawnExistsOptions::Reset)
                    .health_config(HealthConfig::builder().update_interval(Duration::from_secs(1).into()).build())
                    .build(),
            )
            .await?;
            let client_id_spawn = client_id.clone();
            Some(tokio::spawn(async move {
                if let Err(e) = client_actor.start(&mut client_ctx).await {
                    error!("ModelContextProtocolClient actor error: {} for client {}", e, client_id_spawn);
                }
            }))
        } else {
            None
        };
        self.clients.push(ToolClient { hosting, server, client_id, _client_handle: client_handle, tools: vec![] });
        Ok(())
    }

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

    pub async fn list_tools<T: Actor>(&mut self, ctx: &ActorContext<T>) -> Result<Vec<ToolInfo>, ToolsHubError> {
        let mut all_tools = Vec::new();
        for client in &mut self.clients {
            if client.tools.is_empty() {
                match client.list_tools(ctx).await {
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
                if client.health(ctx).await? {
                    all_tools.extend(client.tools.clone());
                } else {
                    error!("Client {} is unhealthy, skipping tools", client.client_id);
                }
            }
        }
        Ok(all_tools)
    }

    pub async fn refresh_tools<T: Actor>(&mut self, ctx: &ActorContext<T>) -> Result<Vec<ToolInfo>, ToolsHubError> {
        let mut all_tools = Vec::new();
        for client in &mut self.clients {
            match client.list_tools(ctx).await {
                Ok(tools) => {
                    client.tools = tools.clone();
                    all_tools.extend(tools);
                }
                Err(e) => error!("Failed to fetch tools: {}", e),
            }
        }
        Ok(all_tools)
    }

    // ---------------------------------------------------------
    // Conversion helper methods (now integrated as associated functions)
    // ---------------------------------------------------------

    fn parse_tool_info(tool: schema::Tool) -> ToolInfo {
        let root_schema = Self::convert_to_root_schema(tool.input_schema.clone()).unwrap();
        ToolInfo::from_schema(tool.name.into(), tool.description.unwrap_or_default().into(), root_schema)
    }

    fn convert_to_root_schema(input: ToolInputSchema) -> Result<RootSchema, anyhow::Error> {
        let schema_obj = Self::convert_schema_object(input)?;
        Ok(RootSchema {
            meta_schema: Some("http://json-schema.org/draft-07/schema#".to_string()),
            schema: schema_obj,
            definitions: Default::default(),
        })
    }

    fn convert_schema_object(input: ToolInputSchema) -> Result<SchemaObject, anyhow::Error> {
        let mut schema_obj = SchemaObject::default();
        schema_obj.instance_type = Some(InstanceType::Object.into());
        if let Some(props) = input.properties {
            let converted_props = Self::convert_properties(props)?;
            schema_obj.object = Some(Box::new(ObjectValidation {
                properties: converted_props,
                required: input.required.unwrap_or_default().into_iter().collect(),
                ..Default::default()
            }));
        }
        Ok(schema_obj)
    }

    fn convert_properties(
        props: BTreeMap<String, BTreeMap<String, Value>>,
    ) -> Result<Map<String, Schema>, anyhow::Error> {
        let mut converted_props = Map::new();
        for (prop_name, prop_value) in props {
            converted_props.insert(prop_name, Schema::Object(Self::convert_property(prop_value)?));
        }
        Ok(converted_props)
    }

    fn convert_property(prop: BTreeMap<String, Value>) -> Result<SchemaObject, anyhow::Error> {
        let mut schema_obj = SchemaObject::default();
        let mut metadata = Metadata::default();
        for (key, value) in prop {
            match key.as_str() {
                "description" => {
                    if let Value::String(desc) = value {
                        metadata.description = Some(desc);
                    }
                }
                "default" => {
                    metadata.default = Some(value);
                }
                "type" => {
                    schema_obj.instance_type = Some(Self::convert_type(value)?);
                }
                "items" => {
                    if let Value::Object(items) = value {
                        let items_schema = Self::convert_property(items.into_iter().collect())?;
                        schema_obj.array = Some(Box::new(ArrayValidation {
                            items: Some(SingleOrVec::Single(Box::new(Schema::Object(items_schema)))),
                            ..Default::default()
                        }));
                    }
                }
                "properties" => {
                    if let Value::Object(nested_props) = value {
                        let converted_nested = Self::convert_nested_properties(nested_props.into_iter().collect())?;
                        schema_obj.object =
                            Some(Box::new(ObjectValidation { properties: converted_nested, ..Default::default() }));
                    }
                }
                "required" => {
                    if let Value::Array(required_fields) = value {
                        if schema_obj.object.is_none() {
                            schema_obj.object = Some(Box::new(ObjectValidation::default()));
                        }
                        if let Some(obj) = &mut schema_obj.object {
                            obj.required =
                                required_fields.into_iter().filter_map(|v| v.as_str().map(String::from)).collect();
                        }
                    }
                }
                "enum" => {
                    if let Value::Array(enum_values) = value {
                        schema_obj.enum_values = Some(enum_values);
                    }
                }
                _ => {}
            }
        }
        schema_obj.metadata = Some(Box::new(metadata));
        Ok(schema_obj)
    }

    fn convert_type(type_value: Value) -> Result<SingleOrVec<InstanceType>, anyhow::Error> {
        match type_value {
            Value::String(type_str) => Ok(SingleOrVec::Single(Box::new(Self::convert_single_type(&type_str)?))),
            Value::Array(types) => {
                let instance_types: Vec<InstanceType> = types
                    .iter()
                    .filter_map(|t| t.as_str().and_then(|type_str| Self::convert_single_type(type_str).ok()))
                    .collect();
                if instance_types.is_empty() {
                    Err(anyhow::anyhow!("No valid types found in array"))
                } else {
                    Ok(SingleOrVec::Vec(instance_types))
                }
            }
            _ => Err(anyhow::anyhow!("Invalid type value")),
        }
    }

    fn convert_single_type(type_str: &str) -> Result<InstanceType, anyhow::Error> {
        match type_str {
            "string" => Ok(InstanceType::String),
            "integer" => Ok(InstanceType::Integer),
            "boolean" => Ok(InstanceType::Boolean),
            "object" => Ok(InstanceType::Object),
            "array" => Ok(InstanceType::Array),
            "number" => Ok(InstanceType::Number),
            "null" => Ok(InstanceType::Null),
            _ => Err(anyhow::anyhow!("Unsupported type: {}", type_str)),
        }
    }

    fn convert_nested_properties(props: Map<String, Value>) -> Result<Map<String, Schema>, anyhow::Error> {
        let mut converted = Map::new();
        for (key, value) in props {
            if let Value::Object(prop_obj) = value {
                let prop_map: BTreeMap<String, Value> = prop_obj.into_iter().collect();
                converted.insert(key, Schema::Object(Self::convert_property(prop_map)?));
            } else {
                let prop_map: BTreeMap<String, Value> = BTreeMap::from_iter([("type".to_string(), value)]);
                converted.insert(key, Schema::Object(Self::convert_property(prop_map)?));
            }
        }
        Ok(converted)
    }
}

#[async_trait]
impl Actor for ToolsHub {
    type Error = ToolsHubError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("ToolsHub actor started: {}", ctx.id());
        let mut stream = ctx.recv().await?;
        while let Some(frame) = stream.next().await {
            if let Some(_list_msg) = frame.is::<ListTools>() {
                match self.list_tools(ctx).await {
                    Ok(tools) => {
                        let res = ListToolsResult { tools, meta: None, next_cursor: None };
                        ctx.reply(res).await?;
                    }
                    Err(e) => {
                        error!("Error listing tools: {}", e);
                        ctx.reply(serde_json::json!({"error": e.to_string()})).await?;
                    }
                }
            } else if let Some(call_msg) = frame.is::<CallTool>() {
                let params = &call_msg.0;
                // Convert the CallToolRequestParams into a ToolCall.
                let tool_call = ToolCall {
                    function: FunctionCall {
                        name: params.name.clone(),
                        arguments: serde_json::Value::Object(params.arguments.clone().unwrap_or_default()),
                    },
                };
                if let Some((_tool_info, client)) = self.get_tool(&params.name) {
                    match client.call(ctx, &tool_call).await {
                        Ok(result) => {
                            ctx.reply(result).await?;
                        }
                        Err(e) => {
                            error!("Error calling tool {}: {}", params.name, e);
                            ctx.reply(serde_json::json!({"error": e.to_string()})).await?;
                        }
                    }
                } else {
                    error!("Tool {} not found", params.name);
                    ctx.reply(serde_json::json!({"error": format!("Tool {} not found", params.name)})).await?;
                }
            }
        }
        info!("ToolsHub actor finished: {}", ctx.id());
        Ok(())
    }
}
