use anyhow::Result;
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
    pub async fn new(engine: &Engine, configs: Vec<ClientConfig>, base_id: String) -> Result<Self, ToolsHubError> {
        let mut hub = Self { clients: vec![] };

        for config in configs {
            let hosting = config.host;
            let server = config.server;
            let client_id = ActorId::of::<ModelContextProtocolClientActor>(format!("{}/{}", base_id, server.name));

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

            hub.clients.push(ToolClient { hosting, server, client_id, _client_handle: client_handle, tools: vec![] });
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

    /// Parses a tool info from a tool schema.
    fn parse_tool_info(tool: schema::Tool) -> ToolInfo {
        let root_schema = Self::convert_to_root_schema(tool.input_schema.clone()).unwrap();
        ToolInfo::from_schema(tool.name.into(), tool.description.unwrap_or_default().into(), root_schema)
    }

    /// Converts a tool input schema to a root schema.
    fn convert_to_root_schema(input: ToolInputSchema) -> Result<RootSchema, anyhow::Error> {
        let schema_obj = Self::convert_schema_object(input)?;
        Ok(RootSchema {
            meta_schema: Some("http://json-schema.org/draft-07/schema#".to_string()),
            schema: schema_obj,
            definitions: Default::default(),
        })
    }

    /// Converts a tool input schema to a schema object.
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

    /// Converts a tool input schema to a schema object.
    fn convert_properties(
        props: BTreeMap<String, BTreeMap<String, Value>>,
    ) -> Result<Map<String, Schema>, anyhow::Error> {
        let mut converted_props = Map::new();
        for (prop_name, prop_value) in props {
            converted_props.insert(prop_name, Schema::Object(Self::convert_property(prop_value)?));
        }
        Ok(converted_props)
    }

    /// Converts a tool input schema to a schema object.
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

    /// Converts a tool input schema to a schema object.
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

    /// Converts a tool input schema to a schema object.
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

    /// Converts a tool input schema to a schema object.
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
            let result = client.call(ctx, &message).await?;
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
        let result = ToolsHub::convert_to_root_schema(input_schema).unwrap();

        // Assert
        let schema = result.schema;
        assert_eq!(schema.instance_type, Some(SingleOrVec::Single(Box::new(InstanceType::Object))));
        assert!(schema.object.is_some());

        let object = schema.object.unwrap();
        let properties = object.properties;

        // Check files array
        let files_schema = properties.get("files").unwrap();
        if let Schema::Object(files_obj) = files_schema {
            assert_eq!(files_obj.instance_type, Some(SingleOrVec::Single(Box::new(InstanceType::Array))));
            assert!(files_obj.array.is_some());

            // Check the items schema of the files array
            let items = files_obj.array.as_ref().unwrap().items.as_ref().unwrap();
            if let SingleOrVec::Single(item_schema) = items {
                if let Schema::Object(item_obj) = item_schema.as_ref() {
                    assert_eq!(item_obj.instance_type, Some(SingleOrVec::Single(Box::new(InstanceType::Object))));

                    // Check file_path and ranges properties
                    let item_properties = item_obj.object.as_ref().unwrap().properties.clone();

                    // Verify file_path
                    let file_path = item_properties.get("file_path").unwrap();
                    if let Schema::Object(file_path_obj) = file_path {
                        assert_eq!(
                            file_path_obj.instance_type,
                            Some(SingleOrVec::Single(Box::new(InstanceType::String)))
                        );
                        assert_eq!(
                            file_path_obj.metadata.as_ref().unwrap().description,
                            Some("Path to the text file.".to_string())
                        );
                    }

                    // Verify ranges
                    let ranges = item_properties.get("ranges").unwrap();
                    if let Schema::Object(ranges_obj) = ranges {
                        assert_eq!(ranges_obj.instance_type, Some(SingleOrVec::Single(Box::new(InstanceType::Array))));

                        // Check ranges items (start/end properties)
                        if let Some(ranges_array) = &ranges_obj.array {
                            if let Some(SingleOrVec::Single(ranges_item)) = &ranges_array.items {
                                if let Schema::Object(ranges_item_obj) = ranges_item.as_ref() {
                                    let range_properties = ranges_item_obj.object.as_ref().unwrap().properties.clone();

                                    // Verify start property
                                    let start = range_properties.get("start").unwrap();
                                    if let Schema::Object(start_obj) = start {
                                        assert_eq!(
                                            start_obj.instance_type,
                                            Some(SingleOrVec::Single(Box::new(InstanceType::Integer)))
                                        );
                                        assert_eq!(
                                            start_obj.metadata.as_ref().unwrap().description,
                                            Some("Starting line number (1-based)".to_string())
                                        );
                                    }

                                    // Verify end property
                                    let end = range_properties.get("end").unwrap();
                                    if let Schema::Object(end_obj) = end {
                                        assert_eq!(
                                            end_obj.instance_type,
                                            Some(SingleOrVec::Vec(vec![InstanceType::Integer, InstanceType::Null]))
                                        );
                                        assert_eq!(
                                            end_obj.metadata.as_ref().unwrap().description,
                                            Some("Ending line number (null for end of file)".to_string())
                                        );
                                    }

                                    // Verify required fields in ranges
                                    assert!(ranges_item_obj.object.as_ref().unwrap().required.contains("start"));
                                }
                            }
                        }
                    }

                    // Verify required fields in items
                    assert!(item_obj.object.as_ref().unwrap().required.contains("file_path"));
                    assert!(item_obj.object.as_ref().unwrap().required.contains("ranges"));
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
        let result = ToolsHub::convert_nested_properties(nested_props).unwrap();

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
