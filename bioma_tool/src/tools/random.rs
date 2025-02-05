use crate::{
    schema::{self, CallToolResult, TextContent},
    tools::{ToolDef, ToolError},
};
use rand::Rng;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Debug, Clone)]
pub struct RandomNumber;

pub const RANDOM_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "start": {
            "description": "The smallest value the random number can be",
            "type": "number"
        },
        "end": {
            "description": "The biggest value the random number can be",
            "type": "number"
        }
    },
    "required": ["start", "end"]
}"#;

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

    fn def() -> schema::Tool {
        let input_schema = serde_json::from_str::<schema::ToolInputSchema>(RANDOM_SCHEMA).unwrap();
        schema::Tool {
            name: Self::NAME.to_string(),
            description: Some(Self::DESCRIPTION.to_string()),
            input_schema,
        }
    }

    async fn call<'a>(&'a self, args: Self::Args) -> Result<CallToolResult, ToolError> {
        let start = args.start;
        let end = args.end;

        if start >= end {
            return Ok(Self::error("Start value must be smaller than end value"));
        }

        let random_number: i32 = rand::thread_rng().gen_range(start..=end);
        Ok(Self::success(format!(
            "Generated number: {}",
            random_number
        )))
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
