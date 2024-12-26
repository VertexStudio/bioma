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
/// This trait provides an interface for executing tools with serialized arguments
/// and returning serialized results.
pub trait ToolCallHandler: Send + Sync {
    /// Executes the tool with the given arguments
    ///
    /// # Arguments
    /// * `args` - Optional map of argument names to JSON values
    ///
    /// # Returns
    /// A future that resolves to either a tool result or an error
    fn call_boxed<'a>(
        &'a self,
        args: Option<BTreeMap<String, Value>>,
    ) -> Pin<Box<dyn Future<Output = Result<CallToolResult, ToolError>> + Send + 'a>>;

    /// Returns the tool's definition/schema
    fn def(&self) -> schema::Tool;
}

/// Trait for defining a concrete tool implementation
///
/// This trait provides a way to define a tool's metadata and implementation
/// with strongly-typed arguments.
pub trait ToolDef: Serialize {
    /// The name of the tool
    const NAME: &'static str;

    /// A description of what the tool does
    const DESCRIPTION: &'static str;

    /// The type representing the tool's input properties
    type Properties: Serialize + JsonSchema + serde::de::DeserializeOwned;

    /// Generates the tool's schema definition
    ///
    /// This method creates a complete tool schema including name, description,
    /// and input parameter definitions derived from the Properties type.
    fn def() -> schema::Tool;

    /// Executes the tool with strongly-typed properties
    ///
    /// # Arguments
    /// * `properties` - The typed input properties for the tool
    ///
    /// # Returns
    /// A future that resolves to either a tool result or an error
    fn call<'a>(
        &'a self,
        properties: Self::Properties,
    ) -> impl Future<Output = Result<CallToolResult, ToolError>> + Send + 'a;
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

            let properties: T::Properties =
                serde_json::from_value(value).map_err(ToolError::ArgumentParse)?;

            self.call(properties).await
        })
    }

    fn def(&self) -> schema::Tool {
        T::def()
    }
}
