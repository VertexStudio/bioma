//! Resource management system for accessing and monitoring data sources.
//!
//! This module provides a framework for:
//! - Defining different types of resources (filesystem, APIs, etc.)
//! - Reading resources with a common interface
//! - Subscribing to resource changes
//! - Notifying clients when resources are updated
//!
//! Resources are identified by URI strings and can provide either text or binary data.
//! The system supports both one-time reads and ongoing subscriptions when resources change.

use crate::schema::{ReadResourceResult, Resource, ResourceTemplate};
use serde::Serialize;
use std::future::Future;
use std::pin::Pin;
use tokio::sync::mpsc;

pub mod filesystem;
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

    /// Error when resource not found
    #[error("Resource not found: {0}")]
    NotFound(String),

    /// Error when trying to subscribe to a resource that doesn't support subscription
    #[error("Resource does not support subscription: {0}")]
    SubscriptionNotSupported(String),
}

/// Trait for handling resource read with dynamic dispatch
pub trait ResourceReadHandler: Send + Sync {
    /// Reads the resource with the given arguments
    fn read_boxed<'a>(
        &'a self,
        uri: String,
    ) -> Pin<Box<dyn Future<Output = Result<ReadResourceResult, ResourceError>> + Send + 'a>>;

    /// Returns the resource's definition/schema
    fn def(&self) -> Resource;

    /// Checks if this handler supports the given URI
    fn supports(&self, uri: &str) -> bool {
        self.def().uri == uri
    }

    /// Returns the resource template if this handler supports templates
    fn template(&self) -> Option<ResourceTemplate> {
        None
    }

    /// Checks if this handler supports subscription for the given resource
    fn supports_subscription(&self, _uri: &str) -> bool {
        false
    }

    /// Subscribe to changes for a resource
    fn subscribe<'a>(
        &'a self,
        _uri: String,
        _on_resource_updated_tx: mpsc::Sender<()>,
    ) -> Pin<Box<dyn Future<Output = Result<(), ResourceError>> + Send + 'a>> {
        Box::pin(async move {
            Err(ResourceError::SubscriptionNotSupported("This resource does not support subscription".to_string()))
        })
    }

    /// Unsubscribe from changes for a resource
    fn unsubscribe<'a>(&'a self, _uri: String) -> Pin<Box<dyn Future<Output = Result<(), ResourceError>> + Send + 'a>> {
        Box::pin(async move {
            Err(ResourceError::SubscriptionNotSupported("This resource does not support subscription".to_string()))
        })
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

    /// The MIME type of the resource (optional)
    const MIME_TYPE: Option<&'static str> = None;

    /// Generates the resource's schema definition
    fn def() -> Resource {
        Resource {
            name: Self::NAME.to_string(),
            description: Some(Self::DESCRIPTION.to_string()),
            uri: Self::URI.to_string(),
            mime_type: Self::MIME_TYPE.map(|mt| mt.to_string()),
            annotations: None,
        }
    }

    /// Generates a resource template definition if this resource supports templates
    fn template() -> Option<ResourceTemplate> {
        None
    }

    /// Reads the resource
    fn read<'a>(&'a self, uri: String) -> impl Future<Output = Result<ReadResourceResult, ResourceError>> + Send + 'a;

    /// Checks if this resource supports subscription
    fn supports_subscription() -> bool {
        false
    }

    /// Subscribe to changes for a resource
    fn subscribe<'a>(
        &'a self,
        _uri: String,
        _on_resource_updated_tx: mpsc::Sender<()>,
    ) -> impl Future<Output = Result<(), ResourceError>> + Send + 'a {
        async move {
            Err(ResourceError::SubscriptionNotSupported("This resource does not support subscription".to_string()))
        }
    }

    /// Unsubscribe from changes for a resource
    fn unsubscribe<'a>(&'a self, _uri: String) -> impl Future<Output = Result<(), ResourceError>> + Send + 'a {
        async move {
            Err(ResourceError::SubscriptionNotSupported("This resource does not support subscription".to_string()))
        }
    }
}

/// Implementation of `ResourceReadHandler` for any type implementing `ResourceDef`
impl<T: ResourceDef + Send + Sync> ResourceReadHandler for T {
    fn read_boxed<'a>(
        &'a self,
        uri: String,
    ) -> Pin<Box<dyn Future<Output = Result<ReadResourceResult, ResourceError>> + Send + 'a>> {
        Box::pin(async move { self.read(uri).await })
    }

    fn def(&self) -> Resource {
        T::def()
    }

    fn supports(&self, uri: &str) -> bool {
        self.def().uri == uri
    }

    fn template(&self) -> Option<ResourceTemplate> {
        T::template()
    }

    fn supports_subscription(&self, _uri: &str) -> bool {
        T::supports_subscription()
    }

    fn subscribe<'a>(
        &'a self,
        uri: String,
        on_resource_updated_tx: mpsc::Sender<()>,
    ) -> Pin<Box<dyn Future<Output = Result<(), ResourceError>> + Send + 'a>> {
        Box::pin(async move { self.subscribe(uri, on_resource_updated_tx).await })
    }

    fn unsubscribe<'a>(&'a self, uri: String) -> Pin<Box<dyn Future<Output = Result<(), ResourceError>> + Send + 'a>> {
        Box::pin(async move { self.unsubscribe(uri).await })
    }
}
