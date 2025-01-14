use crate::user::UserActor;
use anyhow::Result;
use bioma_actor::prelude::*;
use bioma_tool::client::{CallTool, ClientConfig, ListTools, ModelContextProtocolClientActor, ServerConfig};
use bioma_tool::schema::{self, CallToolRequestParams, CallToolResult, ListToolsResult, ToolInputSchema};
use ollama_rs::generation::tools::{ToolCall, ToolInfo};
use schemars::{
    schema::{
        ArrayValidation, InstanceType, Metadata, ObjectValidation, RootSchema, Schema, SchemaObject, SingleOrVec,
    },
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
                SendOptions::builder().timeout(Duration::from_secs(30)).check_health(true).build(),
            )
            .await?;
        info!("Tools from {} ({})", self.server.name, list_tools.tools.len());
        for tool in &list_tools.tools {
            info!("├─ {}", tool.name);
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
                SpawnOptions::builder()
                    .exists(SpawnExistsOptions::Reset)
                    .health_config(
                        HealthConfig::builder()
                            .enabled(true)
                            .update_interval(std::time::Duration::from_secs(1).into())
                            .build(),
                    )
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

    pub async fn list_tools(&mut self, user: &ActorContext<UserActor>) -> Result<Vec<ToolInfo>> {
        let mut all_tools = Vec::new();
        for client in &mut self.clients {
            // Fetch tools if not cached
            if client.tools.is_empty() {
                match client.list_tools(user).await {
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
                // Use cached tools
                all_tools.extend(client.tools.clone());
            }
        }
        Ok(all_tools)
    }

    // Force refresh the cache if needed
    pub async fn refresh_tools(&mut self, user: &ActorContext<UserActor>) -> Result<Vec<ToolInfo>> {
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

fn convert_to_root_schema(input: ToolInputSchema) -> Result<RootSchema> {
    let schema_obj = convert_schema_object(input)?;
    Ok(RootSchema {
        meta_schema: Some("http://json-schema.org/draft-07/schema#".to_string()),
        schema: schema_obj,
        definitions: Default::default(),
    })
}

fn convert_schema_object(input: ToolInputSchema) -> Result<SchemaObject> {
    let mut schema_obj = SchemaObject::default();
    // Set instance type as object
    schema_obj.instance_type = Some(InstanceType::Object.into());
    // Convert properties
    if let Some(props) = input.properties {
        let converted_props = convert_properties(props)?;
        schema_obj.object = Some(Box::new(ObjectValidation {
            properties: converted_props,
            required: input.required.map(|r| r.into_iter().collect()).unwrap_or_default(),
            ..Default::default()
        }));
    }
    Ok(schema_obj)
}

fn convert_properties(props: BTreeMap<String, BTreeMap<String, Value>>) -> Result<Map<String, Schema>> {
    let mut converted_props = Map::new();
    for (prop_name, prop_value) in props {
        converted_props.insert(prop_name, Schema::Object(convert_property(prop_value)?));
    }
    Ok(converted_props)
}

fn convert_property(prop: BTreeMap<String, Value>) -> Result<SchemaObject> {
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
                schema_obj.instance_type = Some(convert_type(value)?);
            }
            "items" => {
                if let Value::Object(items) = value {
                    let items_schema = convert_property(items.into_iter().collect())?;
                    schema_obj.array = Some(Box::new(ArrayValidation {
                        items: Some(SingleOrVec::Single(Box::new(Schema::Object(items_schema)))),
                        ..Default::default()
                    }));
                }
            }
            "properties" => {
                if let Value::Object(nested_props) = value {
                    let converted_nested = convert_nested_properties(nested_props.into_iter().collect::<Map<_, _>>())?;
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

fn convert_type(type_value: Value) -> Result<SingleOrVec<InstanceType>> {
    match type_value {
        Value::String(type_str) => Ok(SingleOrVec::Single(Box::new(convert_single_type(&type_str)?))),
        Value::Array(types) => {
            let instance_types = types
                .iter()
                .filter_map(|t| {
                    t.as_str().and_then(|type_str| match convert_single_type(type_str) {
                        Ok(instance_type) => Some(instance_type),
                        Err(_) => None,
                    })
                })
                .collect::<Vec<_>>();
            if instance_types.is_empty() {
                Err(anyhow::anyhow!("No valid types found in array"))
            } else {
                Ok(SingleOrVec::Vec(instance_types))
            }
        }
        _ => Err(anyhow::anyhow!("Invalid type value")),
    }
}

fn convert_single_type(type_str: &str) -> Result<InstanceType> {
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

fn convert_nested_properties(props: Map<String, Value>) -> Result<Map<String, Schema>> {
    let mut converted = Map::new();

    for (key, value) in props {
        if let Value::Object(prop_obj) = value {
            // Convert the full property object, not just the type
            let prop_map: BTreeMap<String, Value> = prop_obj.into_iter().collect();
            converted.insert(key, Schema::Object(convert_property(prop_map)?));
        } else {
            // Handle case where value might be a direct type specification
            let prop_map: BTreeMap<String, Value> = BTreeMap::from_iter([("type".to_string(), value)]);
            converted.insert(key, Schema::Object(convert_property(prop_map)?));
        }
    }

    Ok(converted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn test_complex_schema_conversion() {
        // Arrange
        let input_schema = ToolInputSchema {
            type_: "object".to_string(),
            properties: Some(BTreeMap::from_iter([
                (
                    "files".to_string(),
                    BTreeMap::from_iter([
                        ("type".to_string(), json!("array")),
                        ("description".to_string(), json!("List of files and their line ranges to read")),
                        (
                            "items".to_string(),
                            json!({
                                "type": "object",
                                "properties": {
                                    "file_path": {
                                        "type": "string",
                                        "description": "Path to the text file."
                                    },
                                    "ranges": {
                                        "type": "array",
                                        "description": "List of line ranges to read from the file",
                                        "items": {
                                            "type": "object",
                                            "properties": {
                                                "start": {
                                                    "type": "integer",
                                                    "description": "Starting line number (1-based)"
                                                },
                                                "end": {
                                                    "type": ["integer", "null"],
                                                    "description": "Ending line number (null for end of file)"
                                                }
                                            },
                                            "required": ["start"]
                                        }
                                    }
                                },
                                "required": ["file_path", "ranges"]
                            }),
                        ),
                    ]),
                ),
                (
                    "encoding".to_string(),
                    BTreeMap::from_iter([
                        ("type".to_string(), json!("string")),
                        ("description".to_string(), json!("Text encoding (default: 'utf-8')")),
                        ("default".to_string(), json!("utf-8")),
                    ]),
                ),
            ])),
            required: Some(vec!["files".to_string()]),
        };
        // Act
        let result = convert_to_root_schema(input_schema).unwrap();
        // Assert
        let schema = result.schema;

        // Check top level structure
        assert_eq!(schema.instance_type, Some(SingleOrVec::Single(Box::new(InstanceType::Object))));
        assert!(schema.object.is_some());

        let object = schema.object.unwrap();
        let properties = object.properties;

        // Check files array
        let files_schema = properties.get("files").unwrap();
        if let Schema::Object(files_obj) = files_schema {
            assert_eq!(files_obj.instance_type, Some(SingleOrVec::Single(Box::new(InstanceType::Array))));
            assert!(files_obj.array.is_some());

            // Check files items (object)
            let files_items = files_obj.array.as_ref().unwrap().items.as_ref().unwrap();
            if let SingleOrVec::Single(item_schema) = files_items {
                if let Schema::Object(item_obj) = item_schema.as_ref() {
                    let item_properties = &item_obj.object.as_ref().unwrap().properties;

                    // Check file_path
                    let file_path = item_properties.get("file_path").unwrap();
                    if let Schema::Object(file_path_obj) = file_path {
                        assert_eq!(
                            file_path_obj.instance_type,
                            Some(SingleOrVec::Single(Box::new(InstanceType::String)))
                        );
                    }

                    // Check ranges array
                    let ranges = item_properties.get("ranges").unwrap();
                    if let Schema::Object(ranges_obj) = ranges {
                        assert_eq!(ranges_obj.instance_type, Some(SingleOrVec::Single(Box::new(InstanceType::Array))));

                        // Check ranges items (object with start and end)
                        let ranges_items = ranges_obj.array.as_ref().unwrap().items.as_ref().unwrap();
                        if let SingleOrVec::Single(range_schema) = ranges_items {
                            if let Schema::Object(range_obj) = range_schema.as_ref() {
                                let range_properties = &range_obj.object.as_ref().unwrap().properties;

                                // Check start field
                                let start = range_properties.get("start").unwrap();
                                if let Schema::Object(start_obj) = start {
                                    assert_eq!(
                                        start_obj.instance_type,
                                        Some(SingleOrVec::Single(Box::new(InstanceType::Integer)))
                                    );
                                }

                                // Check end field (union type)
                                let end = range_properties.get("end").unwrap();
                                if let Schema::Object(end_obj) = end {
                                    if let Some(SingleOrVec::Vec(types)) = &end_obj.instance_type {
                                        assert!(types.contains(&InstanceType::Integer));
                                        assert!(types.contains(&InstanceType::Null));
                                    } else {
                                        panic!("Expected end field to have union type");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // Check encoding field
        let encoding_schema = properties.get("encoding").unwrap();
        if let Schema::Object(encoding_obj) = encoding_schema {
            assert_eq!(encoding_obj.instance_type, Some(SingleOrVec::Single(Box::new(InstanceType::String))));
            assert_eq!(encoding_obj.metadata.as_ref().unwrap().default, Some(json!("utf-8")));
        }
        // Check required fields
        assert!(object.required.contains("files"));
    }
    #[test]
    fn test_nested_property_conversion() {
        // Arrange
        let nested_props = serde_json::from_value::<Map<String, Value>>(json!({
            "test_field": {
                "type": "object",
                "properties": {
                    "nested": {
                        "type": ["integer", "null"],
                        "description": "Test description"
                    }
                }
            }
        }))
        .unwrap();
        // Act
        let result = convert_nested_properties(nested_props).unwrap();
        // Assert
        let test_field = result.get("test_field").unwrap();
        if let Schema::Object(obj) = test_field {
            assert_eq!(obj.instance_type, Some(SingleOrVec::Single(Box::new(InstanceType::Object))));

            let nested_props = &obj.object.as_ref().unwrap().properties;
            let nested = nested_props.get("nested").unwrap();
            if let Schema::Object(nested_obj) = nested {
                if let Some(SingleOrVec::Vec(types)) = &nested_obj.instance_type {
                    assert!(types.contains(&InstanceType::Integer));
                    assert!(types.contains(&InstanceType::Null));
                } else {
                    panic!("Expected nested field to have union type");
                }

                assert_eq!(nested_obj.metadata.as_ref().unwrap().description, Some("Test description".to_string()));
            }
        }
    }
}
