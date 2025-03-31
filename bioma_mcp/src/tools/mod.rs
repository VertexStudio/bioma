use crate::schema::{self, CallToolResult};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

pub mod echo;
pub mod fetch;
pub mod memory;
pub mod random;
pub mod sampling;
pub mod workflow;

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("Failed to parse tool arguments: {0}")]
    ArgumentParse(serde_json::Error),

    #[error("Tool execution failed: {0}")]
    Execution(String),

    #[error("Failed to serialize tool result: {0}")]
    ResultSerialize(serde_json::Error),

    #[error("Custom error: {0}")]
    Custom(String),
}

pub trait ToolCallHandler: Send + Sync {
    fn call_boxed<'a>(
        &'a self,
        args: Option<BTreeMap<String, Value>>,
    ) -> Pin<Box<dyn Future<Output = Result<CallToolResult, ToolError>> + Send + 'a>>;

    fn def(&self) -> schema::Tool;
}

pub trait ToolDef: Serialize {
    const NAME: &'static str;

    const DESCRIPTION: &'static str;

    type Args: Serialize + JsonSchema + serde::de::DeserializeOwned;

    fn def() -> schema::Tool {
        let mut settings = schemars::gen::SchemaSettings::draft07();
        settings.inline_subschemas = true;
        let generator = settings.into_generator();
        let schema = generator.into_root_schema_for::<Self::Args>();

        schema::Tool {
            name: Self::NAME.to_string(),
            description: Some(Self::DESCRIPTION.to_string()),
            input_schema: schema,
            annotations: None,
        }
    }

    fn call<'a>(&'a self, args: Self::Args) -> impl Future<Output = Result<CallToolResult, ToolError>> + Send + 'a;
}

impl<T: ToolDef + Send + Sync> ToolCallHandler for T {
    fn call_boxed<'a>(
        &'a self,
        args: Option<BTreeMap<String, Value>>,
    ) -> Pin<Box<dyn Future<Output = Result<CallToolResult, ToolError>> + Send + 'a>> {
        Box::pin(async move {
            let value = match args {
                Some(map) => serde_json::to_value(map).map_err(ToolError::ArgumentParse)?,
                None => Value::Null,
            };
            let args: T::Args = serde_json::from_value(value).map_err(ToolError::ArgumentParse)?;
            self.call(args).await
        })
    }

    fn def(&self) -> schema::Tool {
        T::def()
    }
}
