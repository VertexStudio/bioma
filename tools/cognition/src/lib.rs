use actix_web::web::Json;
use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use ollama_rs::generation::{
    chat::ChatMessageResponse,
    tools::{ToolCall, ToolInfo},
};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;
pub use tool::ToolsHub;
use tracing::{debug, error, info};
pub use user::UserActor;

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

#[derive(Serialize, Clone)]
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
    tools_hub: Arc<Mutex<ToolsHub>>,
    tx: tokio::sync::mpsc::Sender<Result<Json<ChatResponse>, String>>,
    format: Option<chat::Schema>,
) -> Result<(), ChatToolError> {
    let mut messages = messages.clone();

    // Initial request in non-streaming mode to handle tool calls
    let initial_request = ChatMessages {
        messages: messages.clone(),
        restart: true,
        persist: false,
        stream: false,
        format: format.clone(),
        tools: if tools.is_empty() { None } else { Some(tools.clone()) },
    };

    // Get initial response with tool calls
    let initial_response = match user_actor
        .send_and_wait_reply::<Chat, ChatMessages>(
            initial_request,
            &chat_actor,
            SendOptions::builder().timeout(std::time::Duration::from_secs(60)).build(),
        )
        .await
    {
        Ok(response) => response,
        Err(e) => {
            let _ = tx.send(Err(e.to_string())).await;
            return Err(ChatToolError::SendChatRequestError(e.to_string()));
        }
    };

    // If we have tool calls, execute them first
    if !initial_response.message.tool_calls.is_empty() {
        // Execute tool calls and send their responses
        for tool_call in initial_response.message.tool_calls.iter() {
            match chat_tool_call(user_actor, &tool_call, tools_hub.clone(), tx.clone()).await {
                Ok(tool_response) => {
                    messages.push(ChatMessage::tool(serde_json::to_string(&tool_response).unwrap_or_default()));
                }
                Err(e) => {
                    let _ = tx.send(Err(format!("Tool call failed: {}", e))).await;
                    return Err(e);
                }
            }
        }
    }

    // Always make a final streaming request, whether we had tool calls or not
    let final_request =
        ChatMessages { messages: messages, restart: true, persist: false, stream: true, format, tools: None };

    // Stream final response
    let mut chat_response = match user_actor
        .send::<Chat, ChatMessages>(
            final_request,
            &chat_actor,
            SendOptions::builder().timeout(std::time::Duration::from_secs(60)).build(),
        )
        .await
    {
        Ok(stream) => stream,
        Err(e) => {
            let _ = tx.send(Err(e.to_string())).await;
            return Err(ChatToolError::SendChatRequestError(e.to_string()));
        }
    };

    // Stream only content messages to client
    while let Some(response) = chat_response.next().await {
        match response {
            Ok(chunk) => {
                // Only send chunks that have actual content
                if !chunk.message.content.is_empty() {
                    let response = ChatResponse { response: chunk, context: vec![] };
                    if tx.send(Ok(Json(response))).await.is_err() {
                        return Err(ChatToolError::StreamResponseError("Error streaming response".to_string()));
                    }
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

async fn chat_tool_call(
    user_actor: &ActorContext<UserActor>,
    tool_call: &ToolCall,
    tools_hub: Arc<Mutex<ToolsHub>>,
    tx: tokio::sync::mpsc::Sender<Result<Json<ChatResponse>, String>>,
) -> Result<ToolResponse, ChatToolError> {
    let response = if let Some((_tool_info, tool_client)) = tools_hub.lock().await.get_tool(&tool_call.function.name) {
        // Execute the tool call
        let execution_result = tool_client.call(&user_actor, tool_call).await;

        // Process the result
        let result_json = match execution_result {
            Ok(output) => serde_json::to_value(output.content).unwrap_or_default(),
            Err(e) => serde_json::json!({
                "error": format!("Error calling tool: {:?}", e)
            }),
        };

        // Format tool response
        let formatted_tool_response = ToolResponse {
            server: tool_client.server.name.clone(),
            tool: tool_call.function.name.clone(),
            call: tool_call.clone(),
            response: result_json,
        };

        // Stream tool response
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

        formatted_tool_response
    } else {
        return Err(ChatToolError::ToolNotFound(tool_call.function.name.clone()));
    };

    Ok(response)
}
