use bioma_mcp::{
    schema::{CallToolResult, TextContent},
    server::{Context, RequestContext},
    tools::{ToolDef, ToolError},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ChatMessage {
    #[schemars(description = "Role of the message sender (user, assistant, system)")]
    role: String,

    #[schemars(description = "Content of the message")]
    content: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ChatArgs {
    #[schemars(description = "Array of messages in the conversation")]
    messages: Vec<ChatMessage>,

    #[schemars(description = "Language model to use for chat")]
    model: Option<String>,

    #[schemars(description = "Temperature for generation (0.0-2.0)")]
    temperature: Option<f32>,

    #[schemars(description = "Maximum tokens to generate in the response")]
    max_tokens: Option<u32>,
}

#[derive(Clone)]
pub struct Chat {
    context: Context,
}

impl Chat {
    pub fn new(context: Context) -> Self {
        Self { context }
    }
}

impl ToolDef for Chat {
    const NAME: &'static str = "chat";
    const DESCRIPTION: &'static str = "Chat with an AI model using a conversation history";
    type Args = ChatArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        // In a real implementation, this would connect to the chat actor
        // For now, we'll just return a mock response

        let model = args.model.unwrap_or_else(|| "default-model".to_string());

        // Get the last user message
        let last_user_message = args
            .messages
            .iter()
            .filter(|m| m.role == "user")
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_else(|| "No user message found".to_string());

        let response = format!("Response from model {} to: {}", model, last_user_message);

        Ok(CallToolResult {
            content: vec![
                serde_json::to_value(TextContent { type_: "text".to_string(), text: response, annotations: None })
                    .map_err(ToolError::ResultSerialize)?,
            ],
            is_error: Some(false),
            meta: None,
        })
    }
}
