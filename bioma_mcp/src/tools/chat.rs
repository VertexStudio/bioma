use crate::schema::{CallToolResult, CreateMessageRequestParams, SamplingMessage, TextContent};
use crate::server::Context;
use crate::tools::{ToolDef, ToolError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{error, info};

#[derive(JsonSchema, Serialize, Deserialize)]
pub struct ChatArgs {
    #[schemars(description = "Message to send to the chatbot")]
    pub message: String,
    #[schemars(description = "Maximum number of tokens in the response")]
    #[schemars(default = "default_max_tokens")]
    max_tokens: Option<i64>,

    #[schemars(description = "Temperature for response generation")]
    #[schemars(default = "default_temperature")]
    temperature: Option<f64>,
}

fn default_max_tokens() -> Option<i64> {
    Some(1000)
}

fn default_temperature() -> Option<f64> {
    Some(0.7)
}

#[derive(Clone, Serialize)]
pub struct Chat {
    #[serde(skip)]
    context: Context,
}

impl Chat {
    pub fn new(context: Context) -> Self {
        Self { context }
    }
}

impl ToolDef for Chat {
    const NAME: &'static str = "chat";
    const DESCRIPTION: &'static str = "Chat with an AI assistant";
    type Args = ChatArgs;

    async fn call(&self, args: Self::Args) -> Result<CallToolResult, ToolError> {
        info!("Chat tool called with message: {}", args.message);

        let params = CreateMessageRequestParams {
            messages: vec![SamplingMessage { role: crate::schema::Role::User, content: json!(args.message) }],
            max_tokens: args.max_tokens.unwrap_or(1000),
            temperature: args.temperature,
            system_prompt: Some("You are a helpful AI assistant".to_string()),
            ..Default::default()
        };

        info!("Sending message to LLM");
        match self.context.create_message(params).await {
            Ok(result) => {
                info!("Received response from LLM");
                Ok(CallToolResult {
                    content: vec![serde_json::to_value(TextContent {
                        type_: "text".to_string(),
                        text: result.content.to_string(),
                        annotations: None,
                    })
                    .unwrap()],
                    is_error: Some(false),
                    meta: None,
                })
            }
            Err(e) => {
                error!("Error from LLM: {}", e);
                Ok(CallToolResult {
                    content: vec![serde_json::to_value(TextContent {
                        type_: "text".to_string(),
                        text: format!("Error: {}", e),
                        annotations: None,
                    })
                    .unwrap()],
                    is_error: Some(true),
                    meta: None,
                })
            }
        }
    }
}
