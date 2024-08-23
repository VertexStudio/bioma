use serde::{Deserialize, Serialize};
use crate::error::ActorError;
use crate::actor::ActorId;
use std::future::Future;
use std::any::{Any, type_name};
use std::pin::Pin;
use std::collections::HashMap;

pub trait Actor {

}

pub trait Message: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync {
    type Response: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync;

    fn type_name(&self) -> &'static str {
        type_name::<Self>()
    }

    // async fn send(&self, actor: ActorId) -> Result<Self::Response, ActorError> {
        
    // }
}

pub trait MessageRx<M: Message> {
    fn recv(&self, message: M) -> impl Future<Output = M::Response>;
}


