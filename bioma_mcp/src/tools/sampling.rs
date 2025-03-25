use crate::{
    schema::{CallToolResult, CreateMessageRequestParams, ModelPreferences, TextContent},
    server::Context,
    tools::{ToolDef, ToolError},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::debug;

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
}

impl ToolDef for Sampling {
    const NAME: &'static str = "sampling";
    const DESCRIPTION: &'static str = "Query to ask to the LLM.";
    type Args = SamplingArgs;

    async fn call(&self, _args: Self::Args) -> Result<CallToolResult, ToolError> {
        let model_preferences = Some(ModelPreferences {
            cost_priority: Some(0.5),
            hints: None,
            intelligence_priority: Some(0.7),
            speed_priority: Some(0.4),
        });

        let params = CreateMessageRequestParams {
            include_context: None,
            max_tokens: 100,
            messages: vec![],
            metadata: None,
            model_preferences,
            stop_sequences: None,
            system_prompt: None,
            temperature: Some(0.5),
        };

        debug!("Sending request to client...");

        let sampling = &self.context.create_message(params).await;

        debug!("Got result from client...");

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

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use crate::tools::ToolCallHandler;

//     #[test]
//     fn test_auto_generated_schema() {
//         let tool = Sampling.def();
//         let schema_json = serde_json::to_string_pretty(&tool).unwrap();
//         println!("Tool Schema:\n{}", schema_json);
//     }

//     #[tokio::test]
//     async fn test_random_number_tool() {
//         let tool = Sampling;
//         let args = SamplingArgs { query: "Explain Rust programming language.".to_string() };
//         let result = tool.call(args).await.unwrap();
//         assert!(result.content[0]["text"].as_str().unwrap().contains("Generated number:"));
//     }
// }
