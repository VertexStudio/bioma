use crate::user::UserActor;
use anyhow::Result;
use bioma_actor::prelude::*;
use bioma_tool::client::{
    CallTool, ClientConfig, ListTools, ModelContextProtocolClientActor, PingConfig, ServerConfig,
};
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
use tracing::{error, info};

pub struct ToolClient {
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
}

pub struct Tools {
    pub clients: Vec<ToolClient>,
}

impl Tools {
    pub fn new() -> Self {
        Self { clients: vec![] }
    }

    pub async fn add_tool(
        &mut self,
        engine: &Engine,
        user: &ActorContext<UserActor>,
        config: ClientConfig,
        prefix: String,
    ) -> Result<()> {
        let server = config.server;
        let client_id = ActorId::of::<ModelContextProtocolClientActor>(format!("{}/{}", prefix, server.name));

        let client_handle = if config.host {
            let (mut client_ctx, mut client_actor) = Actor::spawn(
                engine.clone(),
                client_id.clone(),
                ModelContextProtocolClientActor::new(server.clone(), Some(PingConfig::default())),
                SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
            )
            .await?;

            Some(tokio::spawn(async move {
                if let Err(e) = client_actor.start(&mut client_ctx).await {
                    error!("Embeddings actor error: {}", e);
                }
            }))
        } else {
            None
        };

        let list_tools: ListToolsResult = user
            .send_and_wait_reply::<ModelContextProtocolClientActor, ListTools>(
                ListTools(None),
                &client_id,
                SendOptions::builder().timeout(Duration::from_secs(30)).build(),
            )
            .await?;
        info!("Loaded {} tools from {}", list_tools.tools.len(), server.name);
        for tool in &list_tools.tools {
            info!("├─ Tool: {}", tool.name);
        }
        let tools: Vec<ToolInfo> = list_tools.tools.into_iter().map(parse_tool_info).collect();

        self.clients.push(ToolClient { server, client_id, _client_handle: client_handle, tools });

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

    pub fn get_tools(&self) -> Vec<ToolInfo> {
        self.clients.iter().flat_map(|c| c.tools.clone()).collect()
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
