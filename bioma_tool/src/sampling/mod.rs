use crate::schema::{CreateMessageRequest, CreateMessageResult};
use serde::Serialize;
use std::future::Future;
use std::pin::Pin;

/// Errors that can occur during sampling operations
#[derive(Debug, thiserror::Error)]
pub enum SamplingError {
    /// Error when parsing sampling request parameters
    #[error("Failed to parse sampling parameters: {0}")]
    ParameterParse(serde_json::Error),

    /// Error during sampling
    #[error("Sampling failed: {0}")]
    Execution(String),

    /// Error when serializing sampling results to JSON
    #[error("Failed to serialize sampling result: {0}")]
    ResultSerialize(serde_json::Error),

    /// Custom error
    #[error("Custom error: {0}")]
    Custom(String),
}

/// Trait for handling sampling with dynamic dispatch
pub trait SamplingHandler: Send + Sync {
    /// Creates a message using the provided request parameters
    fn create_message_boxed<'a>(
        &'a self,
        request: CreateMessageRequest,
    ) -> Pin<Box<dyn Future<Output = Result<CreateMessageResult, SamplingError>> + Send + 'a>>;
}

/// Trait for defining a concrete sampling implementation
pub trait SamplingDef: Serialize {
    /// Creates a message using the provided request
    fn create_message<'a>(
        &'a self,
        request: CreateMessageRequest,
    ) -> impl Future<Output = Result<CreateMessageResult, SamplingError>> + Send + 'a;
}

/// Implementation of `SamplingHandler` for any type implementing `SamplingDef`
impl<T: SamplingDef + Send + Sync> SamplingHandler for T {
    fn create_message_boxed<'a>(
        &'a self,
        request: CreateMessageRequest,
    ) -> Pin<Box<dyn Future<Output = Result<CreateMessageResult, SamplingError>> + Send + 'a>> {
        Box::pin(async move { self.create_message(request).await })
    }
}
