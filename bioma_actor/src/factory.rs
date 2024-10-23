use crate::prelude::*;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::{sync::RwLock, task::JoinHandle};
use tracing::error;

pub type ActorHandle = JoinHandle<Result<(), SystemActorError>>;

pub trait ActorFactory: Send + Sync {
    fn spawn(
        &self,
        engine: Engine,
        config: serde_json::Value,
        id: ActorId,
        options: SpawnOptions,
    ) -> Result<ActorHandle, SystemActorError>;
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

    pub async fn spawn(
        &self,
        tag: impl Into<Cow<'static, str>>,
        engine: Engine,
        config: serde_json::Value,
        id: ActorId,
        options: SpawnOptions,
    ) -> Result<ActorHandle, SystemActorError> {
        let tag = tag.into();
        let factory = self.map.read().await;
        let factory = factory.get(&tag).ok_or(SystemActorError::ActorTagNotFound(tag))?;
        let res = factory.spawn(engine, config, id, options);
        if let Err(e) = &res {
            panic!("Error spawning actor: {:?}", e);
        }
        res
    }
}

impl std::fmt::Debug for ActorTagRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ActorRegistry")
    }
}
