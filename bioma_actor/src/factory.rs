use crate::prelude::*;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::{sync::RwLock, task::JoinHandle};
use tracing::error;

pub type ActorHandle = Result<JoinHandle<Result<(), SystemActorError>>, SystemActorError>;

pub trait ActorFactory: Send + Sync {
    fn spawn(&self, engine: Engine, config: serde_json::Value, id: ActorId, options: SpawnOptions) -> ActorHandle;
}

#[derive(Default, Clone)]
pub struct ActorTagRegistry {
    pub map: Arc<RwLock<HashMap<Cow<'static, str>, Box<dyn ActorFactory>>>>,
}

impl ActorTagRegistry {
    pub async fn add<F>(&self, tag: impl Into<Cow<'static, str>>, factory: F) -> Result<(), SystemActorError>
    where
        F: ActorFactory + 'static,
    {
        let tag = tag.into();
        let mut map = self.map.write().await;
        if map.contains_key(&tag) {
            error!("Factory already registered for tag: {}", tag);
            return Err(SystemActorError::ActorTagAlreadyRegistered(tag));
        }
        map.insert(tag, Box::new(factory));
        Ok(())
    }
}

impl std::fmt::Debug for ActorTagRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ActorRegistry")
    }
}
