use crate::{
    schema::{CallToolResult, CreateMessageRequestParams, ModelHint, ModelPreferences, SamplingMessage, TextContent},
    server::{Context, RequestContext},
    tools::{ToolDef, ToolError},
};
use futures::StreamExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Clone)]
pub struct Sampling {
    #[serde(skip_serializing)]
    context: Context,
}

impl Sampling {
    pub fn new(context: Context) -> Self {
        Self { context }
    }
}

#[derive(JsonSchema, Serialize, Deserialize)]
struct SamplingMessageSchema {
    pub content: serde_json::Value,
    pub role: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SamplingArgs {
    #[schemars(required = true)]
    #[schemars(description = "Query to ask to the LLM.")]
    #[schemars(with = "Vec<SamplingMessageSchema>")]
    pub messages: Vec<SamplingMessage>,
    #[schemars(required = false)]
    #[schemars(description = "Collection of LLM models to suggest.")]
    #[schemars(with = "Vec<String>")]
    pub models_suggestions: Option<Vec<String>>,
    #[schemars(required = false)]
    #[schemars(description = "Context for the LLM to include.")]
    #[schemars(with = "Vec<String>")]
    pub context: Option<String>,
    #[schemars(required = true)]
    #[schemars(description = "The maximum number of tokens to sample, as requested by the server.")]
    #[schemars(with = "i64")]
    pub max_tokens: i64,
}

impl ToolDef for Sampling {
    const NAME: &'static str = "sampling";
    const DESCRIPTION: &'static str = "Query to ask to the LLM.";
    type Args = SamplingArgs;

    async fn call(&self, args: Self::Args, request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        let mut progress = self.context.track(request_context);
        progress.update_to(0.1, Some("Preparing to sample...".to_string())).await?;
        let hints = match args.models_suggestions {
            Some(models_suggestions) => {
                Some(models_suggestions.iter().map(|name| ModelHint { name: Some(name.clone()) }).collect())
            }
            None => None,
        };

        progress.update_to(0.4, Some("Setting up model preferences...".to_string())).await?;

        let model_preferences = ModelPreferences { hints, ..ModelPreferences::default() };

        let params = CreateMessageRequestParams {
            include_context: args.context,
            max_tokens: args.max_tokens,
            messages: args.messages,
            model_preferences: Some(model_preferences),
            ..CreateMessageRequestParams::default()
        };

        progress.update_to(0.6, Some("Sending request to LLM...".to_string())).await?;

        let mut operation = self.context.create_message(params, true).await?;

        let mut operation_progress = operation.recv();

        tokio::spawn(async move {
            while let Some(progress) = operation_progress.next().await {
                tracing::info!("Progress: {:#?}", progress);
            }
        });

        progress.update_to(0.8, Some("Waiting for response from LLM...".to_string())).await?;

        match operation.await {
            Ok(message_result) => {
                progress.update_to(1.0, Some("Sampling completed".to_string())).await?;
                Ok(Self::success(format!("Sampling result: {:#?}", message_result)))
            }
            Err(e) => {
                progress.update_to(1.0, Some("Sampling failed".to_string())).await?;
                Ok(Self::error(format!("Failed to sample: {}", e)))
            }
        }
    }
}

impl Sampling {
    fn error(error_message: impl Into<String>) -> CallToolResult {
        CallToolResult {
            content: vec![serde_json::to_value(TextContent {
                type_: "text".to_string(),
                text: error_message.into(),
                annotations: None,
            })
            .unwrap_or_default()],
            is_error: Some(true),
            meta: None,
        }
    }

    fn success(message: impl Into<String>) -> CallToolResult {
        CallToolResult {
            content: vec![serde_json::to_value(TextContent {
                type_: "text".to_string(),
                text: message.into(),
                annotations: None,
            })
            .unwrap_or_default()],
            is_error: Some(false),
            meta: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_generated_schema() {
        let tool = Sampling { context: Context::test() };
        let schema_json = serde_json::to_string_pretty(&tool).unwrap();
        println!("Tool Schema:\n{}", schema_json);
    }
}
