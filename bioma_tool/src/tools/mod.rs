use crate::schema::{self, CallToolResult};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

/// Modules containing tool implementations
pub mod echo;
pub mod fetch;
pub mod memory;
pub mod random;

/// Errors that can occur during tool operations
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// Error when parsing tool arguments from JSON
    #[error("Failed to parse tool arguments: {0}")]
    ArgumentParse(serde_json::Error),

    /// Error during tool execution
    #[error("Tool execution failed: {0}")]
    Execution(String),

    /// Error when serializing tool results to JSON
    #[error("Failed to serialize tool result: {0}")]
    ResultSerialize(serde_json::Error),

    /// Error custom
    #[error("Custom error: {0}")]
    Custom(String),
}

/// Trait for handling tool calls with dynamic dispatch
///
/// This trait provides an interface for executing tools with serialized JSON arguments
/// and returning serialized results. It serves as the dynamic dispatch layer for tool execution.
pub trait ToolCallHandler: Send + Sync {
    /// Executes the tool with the given JSON arguments
    ///
    /// # Arguments
    /// * `args` - Optional map of argument names to JSON values. If None, treated as null
    ///
    /// # Returns
    /// A boxed future that resolves to either a `CallToolResult` or a `ToolError`
    fn call_boxed<'a>(
        &'a self,
        args: Option<BTreeMap<String, Value>>,
    ) -> Pin<Box<dyn Future<Output = Result<CallToolResult, ToolError>> + Send + 'a>>;

    /// Returns the tool's schema definition
    ///
    /// # Panics
    /// Currently panics with "Not implemented" as schema generation needs to be implemented
    fn def(&self) -> schema::Tool;
}

/// Trait for defining a concrete tool implementation
///
/// This trait should be implemented by specific tools to provide their functionality
/// with strongly-typed arguments. It works in conjunction with `ToolCallHandler` to
/// provide both type-safe implementation and dynamic dispatch capabilities.
pub trait ToolDef: Serialize {
    /// The name of the tool
    const NAME: &'static str;

    /// A description of what the tool does
    const DESCRIPTION: &'static str;

    /// The type representing the tool's input arguments
    type Args: Serialize + JsonSchema + serde::de::DeserializeOwned;

    /// Generates the tool's schema definition
    ///
    /// Creates a complete tool schema including name, description,
    /// and input parameter definitions based on the `Args` type.
    fn def() -> schema::Tool {
        // let schema = schemars::schema_for!(Self::Args);
        // let input_schema = serde_json::to_value(schema).map_err(|_| "Failed to serialize schema").unwrap();

        let mut settings = schemars::gen::SchemaSettings::draft2019_09();
        settings.inline_subschemas = true;
        let generator = settings.into_generator();
        let schema = generator.into_root_schema_for::<Self::Args>();

        schema::Tool {
            name: Self::NAME.to_string(),
            description: Some(Self::DESCRIPTION.to_string()),
            input_schema: schema,
        }
    }

    /// Executes the tool with strongly-typed arguments
    ///
    /// # Arguments
    /// * `args` - The typed input arguments for the tool, must match `Self::Args`
    ///
    /// # Returns
    /// A future that resolves to either a `CallToolResult` or a `ToolError`
    fn call<'a>(&'a self, args: Self::Args) -> impl Future<Output = Result<CallToolResult, ToolError>> + Send + 'a;
}

/// Implementation of `ToolCallHandler` for any type implementing `ToolDef`
///
/// This provides the bridge between the dynamic dispatch interface and
/// the concrete tool implementation.
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
