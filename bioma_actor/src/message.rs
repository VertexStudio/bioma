use crate::actor::{ActorId, Actor};
use crate::error::ActorError;
use crate::{actor, ActorProtocol};
use serde::{Deserialize, Serialize};
use std::any::{type_name, Any};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

pub trait Message: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync {
    type Response: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync;

    fn type_name(&self) -> &'static str {
        type_name::<Self>()
    }
}

pub trait MessageTx<M: Message> {
}

// pub trait MessageTx<M: Message> {
//     fn send(&self, tx: &ActorId, rx: &ActorId) -> impl Future<Output = Result<M::Response, ActorError>> {
//         async move {
//             // Serialize the message to a JSON Value
//             let message_value = serde_json::to_value(self)?;
//             // Send the message using ActorProtocol
//             let response_value = tx.send(self.type_name(), rx, &message_value).await?;
//             // Deserialize the response
//             let response = serde_json::from_value(response_value)?;
//             Ok(response)
//         }
//     }
// }

pub trait MessageRx<M: Message> {
    fn recv(&self, message: M) -> impl Future<Output = M::Response>;
}

