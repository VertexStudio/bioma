use crate::schema::{self, GetPromptResult};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

/// Modules containing prompt implementations
pub mod greet;

/// Errors that can occur during prompt operations
#[derive(Debug, thiserror::Error)]
pub enum PromptError {
    /// Error when parsing prompt arguments from JSON
    #[error("Failed to parse prompt arguments: {0}")]
    ArgumentParse(serde_json::Error),

    /// Error during prompt execution
    #[error("Prompt execution failed: {0}")]
    Execution(String),

    /// Error when serializing prompt results to JSON
    #[error("Failed to serialize prompt result: {0}")]
    ResultSerialize(serde_json::Error),

    /// Error custom
    #[error("Custom error: {0}")]
    Custom(String),
}

/// Trait for handling prompt get with dynamic dispatch
pub trait PromptGetHandler: Send + Sync {
    /// Executes the prompt with the given arguments
    fn get_boxed<'a>(
        &'a self,
        args: Option<BTreeMap<String, String>>,
    ) -> Pin<Box<dyn Future<Output = Result<GetPromptResult, PromptError>> + Send + 'a>>;

    /// Returns the prompt's definition/schema
    fn def(&self) -> schema::Prompt {
        panic!("Not implemented");
    }
}

/// Trait for defining a concrete prompt implementation
pub trait PromptDef: Serialize {
    /// The name of the prompt
    const NAME: &'static str;

    /// A description of what the prompt does
    const DESCRIPTION: &'static str;

    /// The type representing the prompt's input properties
    type Properties: Serialize + JsonSchema + serde::de::DeserializeOwned;

    /// Generates the prompt's schema definition
    fn def() -> schema::Prompt;

    /// Executes the prompt with strongly-typed properties
    fn get<'a>(
        &'a self,
        properties: Self::Properties,
    ) -> impl Future<Output = Result<GetPromptResult, PromptError>> + Send + 'a;
}

/// Implementation of `PromptGetHandler` for any type implementing `PromptDef`
impl<T: PromptDef + Send + Sync> PromptGetHandler for T {
    fn get_boxed<'a>(
        &'a self,
        args: Option<BTreeMap<String, String>>,
    ) -> Pin<Box<dyn Future<Output = Result<GetPromptResult, PromptError>> + Send + 'a>> {
        Box::pin(async move {
            let value = match args {
                Some(map) => serde_json::to_value(map).map_err(PromptError::ArgumentParse)?,
                None => Value::Null,
            };

            let properties: T::Properties = serde_json::from_value(value).map_err(PromptError::ArgumentParse)?;

            self.get(properties).await
        })
    }

    fn def(&self) -> schema::Prompt {
        T::def()
    }
}
