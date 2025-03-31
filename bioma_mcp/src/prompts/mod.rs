use crate::schema::{self, GetPromptResult};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

pub mod greet;

#[derive(Debug, thiserror::Error)]
pub enum PromptError {
    #[error("Failed to parse prompt arguments: {0}")]
    ArgumentParse(serde_json::Error),

    #[error("Prompt execution failed: {0}")]
    Execution(String),

    #[error("Failed to serialize prompt result: {0}")]
    ResultSerialize(serde_json::Error),

    #[error("Custom error: {0}")]
    Custom(String),
}

pub trait PromptGetHandler: Send + Sync {
    fn get_boxed<'a>(
        &'a self,
        args: Option<BTreeMap<String, String>>,
    ) -> Pin<Box<dyn Future<Output = Result<GetPromptResult, PromptError>> + Send + 'a>>;

    fn def(&self) -> schema::Prompt {
        panic!("Not implemented");
    }

    fn complete_argument_boxed<'a>(
        &'a self,
        argument_name: String,
        argument_value: String,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>, PromptError>> + Send + 'a>>;
}

pub trait PromptCompletionHandler {
    fn complete_argument<'a>(
        &'a self,
        _argument_name: &'a str,
        _argument_value: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>, PromptError>> + Send + 'a>> {
        Box::pin(async move { Ok(vec![]) })
    }
}

pub trait PromptDef: Serialize + PromptCompletionHandler {
    const NAME: &'static str;

    const DESCRIPTION: &'static str;

    type Args: Serialize + JsonSchema + serde::de::DeserializeOwned;

    fn def() -> schema::Prompt;

    fn get<'a>(
        &'a self,
        properties: Self::Args,
    ) -> impl Future<Output = Result<GetPromptResult, PromptError>> + Send + 'a;
}

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
            let properties: T::Args = serde_json::from_value(value).map_err(PromptError::ArgumentParse)?;
            self.get(properties).await
        })
    }

    fn def(&self) -> schema::Prompt {
        T::def()
    }

    fn complete_argument_boxed<'a>(
        &'a self,
        argument_name: String,
        argument_value: String,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>, PromptError>> + Send + 'a>> {
        let name = argument_name.clone();
        let value = argument_value.clone();

        Box::pin(async move { self.complete_argument(&name, &value).await })
    }
}
