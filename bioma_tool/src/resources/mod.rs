use crate::schema::{self, ReadResourceResult};
use serde::Serialize;
use std::future::Future;
use std::pin::Pin;

/// Modules containing resource implementations
pub mod readme;

/// Errors that can occur during resource operations
#[derive(Debug, thiserror::Error)]
pub enum ResourceError {
    /// Error when parsing resource arguments from JSON
    #[error("Failed to parse resource arguments: {0}")]
    ArgumentParse(serde_json::Error),

    /// Error during resource reading
    #[error("Resource reading failed: {0}")]
    Reading(String),

    /// Error when serializing resource results to JSON
    #[error("Failed to serialize resource result: {0}")]
    ResultSerialize(serde_json::Error),

    /// Error custom
    #[error("Custom error: {0}")]
    Custom(String),
}

/// Trait for handling resource read with dynamic dispatch
pub trait ResourceReadHandler: Send + Sync {
    /// Reads the resource with the given arguments
    fn read_boxed<'a>(
        &'a self,
        uri: String,
    ) -> Pin<Box<dyn Future<Output = Result<ReadResourceResult, ResourceError>> + Send + 'a>>;

    /// Returns the resource's definition/schema
    fn def(&self) -> schema::Resource {
        panic!("Not implemented");
    }
}

/// Trait for defining a concrete resource implementation
pub trait ResourceDef: Serialize {
    /// The name of the resource
    const NAME: &'static str;

    /// A description of what the resource does
    const DESCRIPTION: &'static str;

    /// The URI of the resource
    const URI: &'static str;

    /// Generates the resource's schema definition
    fn def() -> schema::Resource;

    /// Reads the resource with strongly-typed arguments
    fn read<'a>(&'a self, uri: String) -> impl Future<Output = Result<ReadResourceResult, ResourceError>> + Send + 'a;
}

/// Implementation of `ResourceReadHandler` for any type implementing `ResourceDef`
impl<T: ResourceDef + Send + Sync> ResourceReadHandler for T {
    fn read_boxed<'a>(
        &'a self,
        uri: String,
    ) -> Pin<Box<dyn Future<Output = Result<ReadResourceResult, ResourceError>> + Send + 'a>> {
        Box::pin(async move { self.read(uri).await })
    }

    fn def(&self) -> schema::Resource {
        T::def()
    }
}
