use std::collections::HashMap;

use actix_web::web::Json;
use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use ollama_rs::generation::{
    chat::ChatMessageResponse,
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

pub async fn chat_with_tools(
    user_actor: &ActorContext<UserActor>,
    chat_actor: &ActorId,
    messages: &Vec<ChatMessage>,
    tools: &Vec<ToolInfo>,
    tool_hub_map: &HashMap<String, ActorId>,
    tx: tokio::sync::mpsc::Sender<Result<Json<ChatResponse>, String>>,
    format: Option<chat::Schema>,
    stream: bool,
) -> Result<(), ChatToolError> {
    // Make chat request with current messages and tools
    let chat_request = ChatMessages {
        messages: messages.clone(),
        restart: true,
        persist: false,
        stream: stream,
        format: format.clone(),
        tools: if tools.is_empty() { None } else { Some(tools.clone()) },
    };

    info!("chat_with_tools: {} tools, {} messages, actor: {}", tools.len(), messages.len(), chat_actor);
    debug!("Chat request: {:#?}", serde_json::to_string_pretty(&chat_request).unwrap_or_default());

    let mut messages = messages.clone();

    // Send chat request
    let mut chat_response = match user_actor
        .send::<Chat, ChatMessages>(
            chat_request,
            &chat_actor,
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

    // Stream response
    let mut is_first_message = true;
    while let Some(response) = chat_response.next().await {
        match response {
            Ok(message_response) => {
                if message_response.message.tool_calls.is_empty() {
                    // Stream the response chunk
                    let response = ChatResponse {
                        response: message_response,
                        context: if is_first_message { messages.clone() } else { vec![] },
                    };
                    is_first_message = false;

                    if tx.send(Ok(Json(response))).await.is_err() {
                        return Err(ChatToolError::StreamResponseError("Error streaming response".to_string()));
                    }
                } else {
                    // We have tool calls, so we need to analyze the dependency tree
                    info!("Tool calls: {:#?}", message_response.message.tool_calls);
                    for tool_call in message_response.message.tool_calls.iter() {
                        // Call the tool
                        let tool_response = chat_tool_call(user_actor, &tool_call, tool_hub_map, tx.clone()).await;
                        match tool_response {
                            Ok(tool_response) => {
                                messages
                                    .push(ChatMessage::tool(serde_json::to_string(&tool_response).unwrap_or_default()));
                            }
                            Err(err) => return Err(err),
                        }
                    }
                    Box::pin(chat_with_tools(
                        user_actor,
                        chat_actor,
                        &messages,
                        tools,
                        tool_hub_map,
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

pub type ToolHubMap = HashMap<String, ActorId>;

async fn chat_tool_call(
    user_actor: &ActorContext<UserActor>,
    tool_call: &ToolCall,
    tool_hub_map: &ToolHubMap,
    tx: tokio::sync::mpsc::Sender<Result<Json<ChatResponse>, String>>,
) -> Result<ToolResponse, ChatToolError> {
    let hub_id = tool_hub_map
        .get(&tool_call.function.name)
        .ok_or_else(|| ChatToolError::ToolNotFound(tool_call.function.name.clone()))?;

    let execution_result = user_actor
        .send_and_wait_reply::<ToolsHub, ToolCall>(
            tool_call.clone(),
            hub_id,
            SendOptions::default(),
        )
        .await
        .map_err(|e| ChatToolError::StreamResponseError(e.to_string()))?;

    // Process the result
    let result_json = match execution_result.is_error {
        Some(true) => serde_json::json!({
            "error": format!("Error calling tool: {:?}", execution_result.content)
        }),
        _ => serde_json::to_value(execution_result.content).unwrap_or_default(),
    };

    // Format tool response
    let tool_response = ToolResponse {
        server: "unknown".to_string(), // We don't have server info anymore
        tool: tool_call.function.name.clone(),
        call: tool_call.clone(),
        response: result_json,
    };

    // Stream tool response
    let message_response = ChatMessageResponse {
        model: "TODO".to_string(),
        created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
        message: ChatMessage::tool(serde_json::to_string(&tool_response).unwrap_or_default()),
        done: false,
        final_data: None,
    };

    let chat_stream_response = ChatResponse { response: message_response, context: vec![] };

    if tx.send(Ok(Json(chat_stream_response))).await.is_err() {
        return Err(ChatToolError::StreamResponseError("Error streaming response".to_string()));
    }

    Ok(tool_response)
}
