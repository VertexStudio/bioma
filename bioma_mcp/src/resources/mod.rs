use crate::schema::{ReadResourceResult, Resource, ResourceTemplate};
use serde::Serialize;
use std::future::Future;
use std::pin::Pin;
use tokio::sync::mpsc;

pub mod filesystem;
pub mod readme;

#[derive(Debug, thiserror::Error)]
pub enum ResourceError {
    #[error("Failed to parse resource arguments: {0}")]
    ArgumentParse(serde_json::Error),

    #[error("Resource reading failed: {0}")]
    Reading(String),

    #[error("Failed to serialize resource result: {0}")]
    ResultSerialize(serde_json::Error),

    #[error("Custom error: {0}")]
    Custom(String),

    #[error("Resource not found: {0}")]
    NotFound(String),

    #[error("Resource does not support subscription: {0}")]
    SubscriptionNotSupported(String),
}

pub trait ResourceReadHandler: Send + Sync {
    fn read_boxed<'a>(
        &'a self,
        uri: String,
    ) -> Pin<Box<dyn Future<Output = Result<ReadResourceResult, ResourceError>> + Send + 'a>>;

    fn def(&self) -> Resource;

    fn supports(&self, uri: &str) -> bool {
        self.def().uri == uri
    }

    fn template(&self) -> Option<ResourceTemplate> {
        None
    }

    fn supports_subscription(&self, _uri: &str) -> bool {
        false
    }

    fn subscribe<'a>(
        &'a self,
        _uri: String,
        _on_resource_updated_tx: mpsc::Sender<()>,
    ) -> Pin<Box<dyn Future<Output = Result<(), ResourceError>> + Send + 'a>> {
        Box::pin(async move {
            Err(ResourceError::SubscriptionNotSupported("This resource does not support subscription".to_string()))
        })
    }

    fn unsubscribe<'a>(&'a self, _uri: String) -> Pin<Box<dyn Future<Output = Result<(), ResourceError>> + Send + 'a>> {
        Box::pin(async move {
            Err(ResourceError::SubscriptionNotSupported("This resource does not support subscription".to_string()))
        })
    }
}

pub trait ResourceDef: Serialize {
    const NAME: &'static str;

    const DESCRIPTION: &'static str;

    const URI: &'static str;

    const MIME_TYPE: Option<&'static str> = None;

    fn def() -> Resource {
        Resource {
            name: Self::NAME.to_string(),
            description: Some(Self::DESCRIPTION.to_string()),
            uri: Self::URI.to_string(),
            mime_type: Self::MIME_TYPE.map(|mt| mt.to_string()),
            annotations: None,
        }
    }

    fn template() -> Option<ResourceTemplate> {
        None
    }

    fn read<'a>(&'a self, uri: String) -> impl Future<Output = Result<ReadResourceResult, ResourceError>> + Send + 'a;

    fn supports_subscription() -> bool {
        false
    }

    fn subscribe<'a>(
        &'a self,
        _uri: String,
        _on_resource_updated_tx: mpsc::Sender<()>,
    ) -> impl Future<Output = Result<(), ResourceError>> + Send + 'a {
        async move {
            Err(ResourceError::SubscriptionNotSupported("This resource does not support subscription".to_string()))
        }
    }

    fn unsubscribe<'a>(&'a self, _uri: String) -> impl Future<Output = Result<(), ResourceError>> + Send + 'a {
        async move {
            Err(ResourceError::SubscriptionNotSupported("This resource does not support subscription".to_string()))
        }
    }
}

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
