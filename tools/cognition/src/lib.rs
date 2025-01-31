use actix_web::web::Json;
use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use bioma_tool::{
    client::ModelContextProtocolClientActor,
    schema::{CallToolRequestParams, CallToolResult, ListToolsResult},
};
use chrono;
use futures_util::StreamExt;
use ollama_rs::generation::{
    chat::{ChatMessage, ChatMessageResponse},
    tools::{ToolCall, ToolInfo},
};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;
pub use tool::ToolsHub;
use tracing::{debug, error, info};

pub use user::UserActor;

pub mod health_check;
pub mod tool;
pub mod user;

#[derive(Error, Debug)]
pub enum ChatToolError {
    #[error("Error fetching tools: {0}")]
    FetchToolsError(String),
    #[error("Error sending chat request: {0}")]
    SendChatRequestError(String),
    #[error("Error streaming response: {0}")]
    StreamResponseError(String),
    #[error("Error analyzing dependency tree: {0}")]
    AnalyzeDependencyTreeError(String),
    #[error("Tool not found: {0}")]
    ToolNotFound(String),
}

#[derive(Debug, Serialize)]
pub struct ChatResponse {
    #[serde(flatten)]
    pub response: ChatMessageResponse,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<ChatMessage>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ToolResponse {
    pub server: String,
    pub tool: String,
    pub call: ToolCall,
    pub response: Value,
}

/// Free function to call a tool via a given actor id.
pub async fn call_tool(
    user: &ActorContext<UserActor>,
    client_id: &ActorId,
    tool_call: &ToolCall,
) -> Result<CallToolResult, anyhow::Error> {
    let args: std::collections::BTreeMap<String, Value> = tool_call
        .function
        .arguments
        .as_object()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    let request = CallToolRequestParams { name: tool_call.function.name.clone(), arguments: Some(args) };
    let response = user
        .send_and_wait_reply::<ModelContextProtocolClientActor, bioma_tool::client::CallTool>(
            bioma_tool::client::CallTool(request),
            client_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(30)).check_health(true).build(),
        )
        .await?;
    Ok(response)
}

/// Free function to list tools via a given actor id.
/// The ServerConfig is used for logging purposes.
pub async fn list_tools(
    user: &ActorContext<UserActor>,
    client_id: &ActorId,
    server: &bioma_tool::client::ServerConfig,
) -> Result<Vec<ToolInfo>, anyhow::Error> {
    let list_tools: ListToolsResult = user
        .send_and_wait_reply::<ModelContextProtocolClientActor, bioma_tool::client::ListTools>(
            bioma_tool::client::ListTools(None),
            client_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(30)).check_health(true).build(),
        )
        .await?;
    info!("Tools from {} ({})", server.name, list_tools.tools.len());
    for tool in &list_tools.tools {
        info!("├─ {}", tool.name);
    }
    Ok(list_tools.tools.into_iter().map(parse_tool_info).collect())
}

fn parse_tool_info(tool: bioma_tool::schema::Tool) -> ToolInfo {
    let root_schema = convert_to_root_schema(tool.input_schema.clone()).unwrap();
    ToolInfo::from_schema(tool.name.clone().into(), tool.description.clone().unwrap_or_default().into(), root_schema)
}

fn convert_to_root_schema(
    input: bioma_tool::schema::ToolInputSchema,
) -> Result<schemars::schema::RootSchema, anyhow::Error> {
    let schema_obj = convert_schema_object(input)?;
    Ok(schemars::schema::RootSchema {
        meta_schema: Some("http://json-schema.org/draft-07/schema#".to_string()),
        schema: schema_obj,
        definitions: Default::default(),
    })
}

fn convert_schema_object(
    input: bioma_tool::schema::ToolInputSchema,
) -> Result<schemars::schema::SchemaObject, anyhow::Error> {
    let mut schema_obj = schemars::schema::SchemaObject::default();
    schema_obj.instance_type = Some(schemars::schema::InstanceType::Object.into());
    if let Some(props) = input.properties {
        let converted_props = convert_properties(props)?;
        schema_obj.object = Some(Box::new(schemars::schema::ObjectValidation {
            properties: converted_props,
            required: input.required.unwrap_or_default().into_iter().collect(),
            ..Default::default()
        }));
    }
    Ok(schema_obj)
}

fn convert_properties(
    props: std::collections::BTreeMap<String, std::collections::BTreeMap<String, Value>>,
) -> Result<schemars::Map<String, schemars::schema::Schema>, anyhow::Error> {
    let mut converted_props = schemars::Map::new();
    for (prop_name, prop_value) in props {
        converted_props.insert(prop_name, schemars::schema::Schema::Object(convert_property(prop_value)?));
    }
    Ok(converted_props)
}

fn convert_property(
    prop: std::collections::BTreeMap<String, Value>,
) -> Result<schemars::schema::SchemaObject, anyhow::Error> {
    let mut schema_obj = schemars::schema::SchemaObject::default();
    let mut metadata = schemars::schema::Metadata::default();
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
                    schema_obj.array = Some(Box::new(schemars::schema::ArrayValidation {
                        items: Some(schemars::schema::SingleOrVec::Single(Box::new(schemars::schema::Schema::Object(
                            items_schema,
                        )))),
                        ..Default::default()
                    }));
                }
            }
            "properties" => {
                if let Value::Object(nested_props) = value {
                    let converted_nested = convert_nested_properties(nested_props.into_iter().collect())?;
                    schema_obj.object = Some(Box::new(schemars::schema::ObjectValidation {
                        properties: converted_nested,
                        ..Default::default()
                    }));
                }
            }
            "required" => {
                if let Value::Array(required_fields) = value {
                    if schema_obj.object.is_none() {
                        schema_obj.object = Some(Box::new(schemars::schema::ObjectValidation::default()));
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

fn convert_type(
    type_value: Value,
) -> Result<schemars::schema::SingleOrVec<schemars::schema::InstanceType>, anyhow::Error> {
    match type_value {
        Value::String(type_str) => Ok(schemars::schema::SingleOrVec::Single(Box::new(convert_single_type(&type_str)?))),
        Value::Array(types) => {
            let instance_types = types
                .iter()
                .filter_map(|t| t.as_str().and_then(|type_str| convert_single_type(type_str).ok()))
                .collect::<Vec<_>>();
            if instance_types.is_empty() {
                Err(anyhow::anyhow!("No valid types found in array"))
            } else {
                Ok(schemars::schema::SingleOrVec::Vec(instance_types))
            }
        }
        _ => Err(anyhow::anyhow!("Invalid type value")),
    }
}

fn convert_single_type(type_str: &str) -> Result<schemars::schema::InstanceType, anyhow::Error> {
    match type_str {
        "string" => Ok(schemars::schema::InstanceType::String),
        "integer" => Ok(schemars::schema::InstanceType::Integer),
        "boolean" => Ok(schemars::schema::InstanceType::Boolean),
        "object" => Ok(schemars::schema::InstanceType::Object),
        "array" => Ok(schemars::schema::InstanceType::Array),
        "number" => Ok(schemars::schema::InstanceType::Number),
        "null" => Ok(schemars::schema::InstanceType::Null),
        _ => Err(anyhow::anyhow!("Unsupported type: {}", type_str)),
    }
}

fn convert_nested_properties(
    props: schemars::Map<String, Value>,
) -> Result<schemars::Map<String, schemars::schema::Schema>, anyhow::Error> {
    let mut converted = schemars::Map::new();
    for (key, value) in props {
        if let Value::Object(prop_obj) = value {
            let prop_map: std::collections::BTreeMap<String, Value> = prop_obj.into_iter().collect();
            converted.insert(key, schemars::schema::Schema::Object(convert_property(prop_map)?));
        } else {
            let prop_map: std::collections::BTreeMap<String, Value> =
                std::collections::BTreeMap::from([("type".to_string(), value)]);
            converted.insert(key, schemars::schema::Schema::Object(convert_property(prop_map)?));
        }
    }
    Ok(converted)
}

/// --- NEW FUNCTIONS FOR ACTOR-BASED TOOLS --- ///

/// A variant of chat_with_tools that uses a provided vector of (ToolInfo, ActorId) pairs.
pub async fn chat_with_actor_tools(
    user_actor: &ActorContext<UserActor>,
    chat_actor: &ActorId,
    messages: &Vec<ChatMessage>,
    actor_tools: &Vec<(ToolInfo, ActorId)>,
    tx: tokio::sync::mpsc::Sender<Result<Json<ChatResponse>, String>>,
    format: Option<chat::Schema>,
    stream: bool,
) -> Result<(), ChatToolError> {
    let chat_request = ChatMessages {
        messages: messages.clone(),
        restart: true,
        persist: false,
        stream,
        format: format.clone(),
        tools: if actor_tools.is_empty() { None } else { Some(actor_tools.iter().map(|(ti, _)| ti.clone()).collect()) },
    };

    info!("chat_with_actor_tools: {} tools, {} messages, actor: {}", actor_tools.len(), messages.len(), chat_actor);
    debug!("Chat request: {:#?}", serde_json::to_string_pretty(&chat_request).unwrap_or_default());

    let mut messages_clone = messages.clone();
    let mut chat_response = match user_actor
        .send::<Chat, ChatMessages>(
            chat_request,
            chat_actor,
            SendOptions::builder().timeout(std::time::Duration::from_secs(2000)).build(),
        )
        .await
    {
        Ok(stream) => stream,
        Err(e) => {
            let _ = tx.send(Err(e.to_string())).await;
            return Err(ChatToolError::SendChatRequestError(e.to_string()));
        }
    };

    let mut is_first_message = true;
    while let Some(response) = chat_response.next().await {
        match response {
            Ok(message_response) => {
                if message_response.message.tool_calls.is_empty() {
                    info!("Chat response: {:#?}", message_response);
                    let response_to_send = ChatResponse {
                        response: message_response,
                        context: if is_first_message { messages_clone.clone() } else { vec![] },
                    };
                    is_first_message = false;
                    if tx.send(Ok(Json(response_to_send))).await.is_err() {
                        return Err(ChatToolError::StreamResponseError("Error streaming response".to_string()));
                    }
                } else {
                    info!("Tool calls: {:#?}", message_response.message.tool_calls);
                    for tool_call in message_response.message.tool_calls.iter() {
                        let tool_response =
                            chat_actor_tool_call(user_actor, tool_call, actor_tools, tx.clone()).await?;
                        messages_clone
                            .push(ChatMessage::tool(serde_json::to_string(&tool_response).unwrap_or_default()));
                    }
                    Box::pin(chat_with_actor_tools(
                        user_actor,
                        chat_actor,
                        &messages_clone,
                        actor_tools,
                        tx.clone(),
                        format.clone(),
                        stream,
                    ))
                    .await?
                }
            }
            Err(e) => {
                let _ = tx.send(Err(e.to_string())).await;
                return Err(ChatToolError::StreamResponseError(e.to_string()));
            }
        }
    }
    Ok(())
}

async fn chat_actor_tool_call(
    user_actor: &ActorContext<UserActor>,
    tool_call: &ToolCall,
    actor_tools: &Vec<(ToolInfo, ActorId)>,
    tx: tokio::sync::mpsc::Sender<Result<Json<ChatResponse>, String>>,
) -> Result<ToolResponse, ChatToolError> {
    let maybe = actor_tools.iter().find(|(ti, _)| ti.name() == tool_call.function.name).cloned();
    let (tool_info, actor_id) = maybe.ok_or(ChatToolError::ToolNotFound(tool_call.function.name.clone()))?;
    let execution_result = call_tool(user_actor, &actor_id, tool_call).await;
    let result_json = match execution_result {
        Ok(output) => serde_json::to_value(output.content).unwrap_or_default(),
        Err(e) => serde_json::json!({
            "error": format!("Error calling tool: {:?}", e)
        }),
    };

    let formatted_tool_response = ToolResponse {
        server: actor_id.to_string(),
        tool: tool_call.function.name.clone(),
        call: tool_call.clone(),
        response: result_json,
    };

    let response = ChatMessageResponse {
        model: "TODO".to_string(),
        created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
        message: ChatMessage::tool(serde_json::to_string(&formatted_tool_response).unwrap_or_default()),
        done: false,
        final_data: None,
    };

    let chat_stream_response = ChatResponse { response: response.clone(), context: vec![] };

    if tx.send(Ok(Json(chat_stream_response))).await.is_err() {
        return Err(ChatToolError::StreamResponseError("Error streaming response".to_string()));
    }

    Ok(formatted_tool_response)
}
