use crate::{
    schema::{CallToolResult, TextContent},
    server::RequestContext,
    tools::{ToolDef, ToolError},
};
use rand::Rng;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Debug, Clone)]
pub struct RandomNumber;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct RandomNumberArgs {
    #[schemars(required = true)]
    #[schemars(description = "The smaller value the random number can be")]
    #[schemars(with = "i32")]
    pub start: i32,
    #[schemars(required = true)]
    #[schemars(description = "The biggest value the random number can be")]
    #[schemars(with = "i32")]
    pub end: i32,
}

impl ToolDef for RandomNumber {
    const NAME: &'static str = "random";
    const DESCRIPTION: &'static str = "Generate a random number";
    type Args = RandomNumberArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        let start = args.start;
        let end = args.end;

        if start >= end {
            return Ok(Self::error("Start value must be smaller than end value"));
        }

        let random_number: i32 = rand::thread_rng().gen_range(start..=end);

        if random_number > end || random_number < start {
            return Ok(Self::error("Invalid value"));
        }

        Ok(Self::success(format!("Generated number: {}", random_number)))
    }
}

impl RandomNumber {
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
    use crate::tools::ToolCallHandler;

    #[test]
    fn test_auto_generated_schema() {
        let tool = RandomNumber.def();
        let schema_json = serde_json::to_string_pretty(&tool).unwrap();
        println!("Tool Schema:\n{}", schema_json);
    }

    #[tokio::test]
    async fn test_random_number_tool() {
        let tool = RandomNumber;
        let args = RandomNumberArgs { start: 1, end: 10 };
        let result = tool.call(args, RequestContext::default()).await.unwrap();
        assert!(result.content[0]["text"].as_str().unwrap().contains("Generated number:"));
    }
}
