use crate::schema::{ListRootsRequest, ListRootsResult};
use serde::Serialize;
use std::future::Future;
use std::pin::Pin;

/// Module containing roots implementations
pub mod filesystem;
/// Module for readme root implementation
pub mod readme;

/// Errors that can occur during roots operations
#[derive(Debug, thiserror::Error)]
pub enum RootsError {
    /// Error when parsing roots request parameters
    #[error("Failed to parse roots parameters: {0}")]
    ParameterParse(serde_json::Error),

    /// Error during roots operation
    #[error("Roots operation failed: {0}")]
    Execution(String),

    /// Error when serializing roots results to JSON
    #[error("Failed to serialize roots result: {0}")]
    ResultSerialize(serde_json::Error),

    /// Access error for roots
    #[error("Access error: {0}")]
    AccessError(String),

    /// Custom error
    #[error("Custom error: {0}")]
    Custom(String),
}

/// Trait for handling roots operations with dynamic dispatch
pub trait RootsHandler: Send + Sync {
    /// Lists available roots
    fn list_roots_boxed<'a>(
        &'a self,
        request: ListRootsRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ListRootsResult, RootsError>> + Send + 'a>>;
}

/// Trait for defining a concrete roots implementation
pub trait RootsDef: Serialize {
    /// Lists available roots
    fn list_roots<'a>(
        &'a self,
        request: ListRootsRequest,
    ) -> impl Future<Output = Result<ListRootsResult, RootsError>> + Send + 'a;
}

/// Implementation of `RootsHandler` for any type implementing `RootsDef`
impl<T: RootsDef + Send + Sync> RootsHandler for T {
    fn list_roots_boxed<'a>(
        &'a self,
        request: ListRootsRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ListRootsResult, RootsError>> + Send + 'a>> {
        Box::pin(async move { self.list_roots(request).await })
    }
}
