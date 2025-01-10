use crate::user::UserActor;
use anyhow::Result;
use bioma_actor::prelude::*;
use bioma_tool::client::{CallTool, ClientConfig, ListTools, ModelContextProtocolClientActor, ServerConfig};
use bioma_tool::schema::{self, CallToolRequestParams, CallToolResult, ListToolsResult, ToolInputSchema};
use ollama_rs::generation::tools::{ToolCall, ToolInfo};
use schemars::{
    schema::{InstanceType, Metadata, ObjectValidation, RootSchema, Schema, SchemaObject, SingleOrVec},
    Map,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

pub struct ToolClient {
    pub hosting: bool,
    pub server: ServerConfig,
    pub client_id: ActorId,
    pub _client_handle: Option<JoinHandle<()>>,
    pub tools: Vec<ToolInfo>,
}

impl ToolClient {
    pub async fn call(&self, user: &ActorContext<UserActor>, tool_call: &ToolCall) -> Result<CallToolResult> {
        let args: BTreeMap<String, Value> = tool_call
            .function
            .arguments
            .as_object()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        let request = CallToolRequestParams { name: tool_call.function.name.clone(), arguments: Some(args) };
        let response = user
            .send_and_wait_reply::<ModelContextProtocolClientActor, CallTool>(
                CallTool(request),
                &self.client_id,
                SendOptions::builder().timeout(Duration::from_secs(30)).build(),
            )
            .await?;
        Ok(response)
    }

    pub async fn list_tools(&self, user: &ActorContext<UserActor>) -> Result<Vec<ToolInfo>> {
        let list_tools: ListToolsResult = user
            .send_and_wait_reply::<ModelContextProtocolClientActor, ListTools>(
                ListTools(None),
                &self.client_id,
                SendOptions::builder().timeout(Duration::from_secs(30)).build(),
            )
            .await?;
        info!("Listed {} tools from {}", list_tools.tools.len(), self.server.name);
        for tool in &list_tools.tools {
            info!("├─ Tool: {}", tool.name);
        }
        Ok(list_tools.tools.into_iter().map(parse_tool_info).collect())
    }
}

pub struct Tools {
    pub clients: Vec<ToolClient>,
}

impl Tools {
    pub fn new() -> Self {
        Self { clients: vec![] }
    }

    pub async fn add_tool(&mut self, engine: &Engine, config: ClientConfig, prefix: String) -> Result<()> {
        let hosting = config.host;
        let server = config.server;
        let client_id = ActorId::of::<ModelContextProtocolClientActor>(format!("{}/{}", prefix, server.name));

        // If hosting, spawn client, which will spawn and host a ModelContextProtocol server
        let client_handle = if config.host {
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

    pub async fn list_tools(&mut self, user: &ActorContext<UserActor>) -> Result<Vec<ToolInfo>> {
        let mut all_tools = Vec::new();
        for client in &mut self.clients {
            match client.list_tools(user).await {
                Ok(tools) => {
                    client.tools = tools.clone();
                    all_tools.extend(tools);
                }
                Err(e) => error!("Failed to fetch tools: {}", e),
            }
        }
        Ok(all_tools)
    }
}

fn parse_tool_info(tool: schema::Tool) -> ToolInfo {
    let root_schema = convert_to_root_schema(tool.input_schema.clone()).unwrap();
    ToolInfo::from_schema(tool.name.clone().into(), tool.description.clone().unwrap_or_default().into(), root_schema)
}

fn convert_to_root_schema(input: ToolInputSchema) -> anyhow::Result<RootSchema> {
    let mut schema_obj = SchemaObject::default();

    // Set instance type as object
    schema_obj.instance_type = Some(InstanceType::Object.into());

    // Convert properties
    if let Some(props) = input.properties {
        let mut converted_props = Map::new();

        for (prop_name, prop_value) in props {
            let mut prop_schema = SchemaObject::default();

            // Handle description and type from property values
            for (key, value) in prop_value {
                match key.as_str() {
                    "description" => {
                        if let serde_json::Value::String(desc) = value {
                            prop_schema.metadata =
                                Some(Box::new(Metadata { description: Some(desc), ..Default::default() }));
                        }
                    }
                    "type" => match value {
                        serde_json::Value::String(type_str) => {
                            prop_schema.instance_type = Some(match type_str.as_str() {
                                "string" => InstanceType::String.into(),
                                "integer" => InstanceType::Integer.into(),
                                "boolean" => InstanceType::Boolean.into(),
                                "object" => InstanceType::Object.into(),
                                "array" => InstanceType::Array.into(),
                                "number" => InstanceType::Number.into(),
                                "null" => InstanceType::Null.into(),
                                _ => return Err(anyhow::anyhow!("Unsupported type: {}", type_str)),
                            });
                        }
                        // TODO: Ollama doesn't support this
                        serde_json::Value::Array(types) => {
                            let instance_types: Vec<InstanceType> = types
                                .iter()
                                .filter_map(|t| {
                                    if let serde_json::Value::String(type_str) = t {
                                        Some(match type_str.as_str() {
                                            "string" => InstanceType::String,
                                            "integer" => InstanceType::Integer,
                                            "boolean" => InstanceType::Boolean,
                                            "object" => InstanceType::Object,
                                            "array" => InstanceType::Array,
                                            "number" => InstanceType::Number,
                                            "null" => InstanceType::Null,
                                            _ => return None,
                                        })
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            prop_schema.instance_type = Some(SingleOrVec::Vec(instance_types));
                        }
                        _ => return Err(anyhow::anyhow!("Invalid type value")),
                    },
                    "default" => {
                        if prop_schema.metadata.is_none() {
                            prop_schema.metadata = Some(Box::new(Metadata::default()));
                        }
                        if let Some(metadata) = &mut prop_schema.metadata {
                            metadata.default = Some(value);
                        }
                    }
                    "enum" => {
                        if let serde_json::Value::Array(enum_values) = value {
                            prop_schema.enum_values = Some(enum_values);
                        }
                    }
                    _ => {}
                }
            }

            converted_props.insert(prop_name, Schema::Object(prop_schema));
        }

        schema_obj.object = Some(Box::new(ObjectValidation { properties: converted_props, ..Default::default() }));
    }

    // Set required fields
    if let Some(required_fields) = input.required {
        if schema_obj.object.is_none() {
            schema_obj.object = Some(Box::new(ObjectValidation::default()));
        }
        if let Some(obj) = &mut schema_obj.object {
            obj.required = required_fields.into_iter().collect();
        }
    }

    Ok(RootSchema {
        meta_schema: Some("http://json-schema.org/draft-07/schema#".to_string()),
        schema: schema_obj,
        definitions: Default::default(),
    })
}
