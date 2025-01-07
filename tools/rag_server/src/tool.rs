use anyhow::Result;
use bioma_tool::client::ServerConfig;
use bioma_tool::schema::{CallToolRequestParams, CallToolResult, ToolInputSchema};
use bioma_tool::ModelContextProtocolClient;
use ollama_rs::generation::tools::{ToolCall, ToolInfo};
use schemars::{
    schema::{InstanceType, Metadata, ObjectValidation, RootSchema, Schema, SchemaObject, SingleOrVec},
    Map,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

pub struct ToolClient {
    pub server: ServerConfig,
    pub client: Arc<Mutex<ModelContextProtocolClient>>,
    pub tools: Vec<ToolInfo>,
}

impl ToolClient {
    pub async fn call(&self, tool_call: &ToolCall) -> Result<CallToolResult> {
        let args: BTreeMap<String, Value> = tool_call
            .function
            .arguments
            .as_object()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        let request = CallToolRequestParams { name: tool_call.function.name.clone(), arguments: Some(args) };
        self.client.lock().await.call_tool(request).await
    }
}

pub struct Tools {
    pub clients: Vec<ToolClient>,
}

impl Tools {
    pub fn new() -> Self {
        Self { clients: vec![] }
    }

    pub async fn add_server(&mut self, server: ServerConfig) -> Result<()> {
        let mut client = ModelContextProtocolClient::new(server.clone()).await?;
        let list_tools = client.list_tools(None).await?;
        info!("Loaded {} tools from {}", list_tools.tools.len(), server.name);
        for tool in &list_tools.tools {
            info!("├─ Tool: {}", tool.name);
        }
        let tools: Vec<ToolInfo> = list_tools.tools.into_iter().map(parse_tool_info).collect();
        // Save tools info to file for debugging
        // let tools_json = serde_json::to_string_pretty(&tools)?;
        // let debug_path = format!("debug_tools_{}.json", server.name.replace(" ", "_"));
        // std::fs::write(&debug_path, tools_json)?;
        // info!("Saved tools debug info to {}", debug_path);
        self.clients.push(ToolClient { server, client: Arc::new(Mutex::new(client)), tools });
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

fn parse_tool_info(tool: bioma_tool::schema::Tool) -> ToolInfo {
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
