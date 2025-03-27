use crate::{
    schema::{CallToolResult, CreateMessageRequestParams, ModelHint, ModelPreferences, SamplingMessage, TextContent},
    server::Context,
    tools::{ToolDef, ToolError},
};
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

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SamplingArgs {
    #[schemars(required = true)]
    #[schemars(description = "Query to ask to the LLM.")]
    #[schemars(with = "String")]
    pub query: String,
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

    async fn call(&self, args: Self::Args) -> Result<CallToolResult, ToolError> {
        let hints = match args.models_suggestions {
            Some(models_suggestions) => {
                Some(models_suggestions.iter().map(|name| ModelHint { name: Some(name.clone()) }).collect())
            }
            None => None,
        };

        let model_preferences = ModelPreferences { hints, ..ModelPreferences::default() };

        let params = CreateMessageRequestParams {
            include_context: args.context,
            max_tokens: args.max_tokens,
            messages: vec![SamplingMessage {
                content: match serde_json::to_value(&args.query) {
                    Ok(value) => value,
                    Err(e) => return Ok(Self::error(format!("Failed to serialize query: {}", e))),
                },
                role: crate::schema::Role::Assistant,
            }],
            model_preferences: Some(model_preferences),
            ..CreateMessageRequestParams::default()
        };

        let sampling = &self.context.create_message(params).await;

        match sampling {
            Ok(message_result) => Ok(Self::success(format!("Sampling result: {:#?}", message_result))),
            Err(e) => Ok(Self::error(format!("Failed to sample: {}", e))),
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
