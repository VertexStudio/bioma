use crate::prelude::*;
use serde::{Deserialize, Serialize};


/// A relay actor for sending messages to another actor from outside the actor system
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Relay;

impl Actor for Relay {
    type Error = SystemActorError;

    fn start(&mut self, _ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), Self::Error>> {
        async move {
            panic!("Relay should not be started");
        }
    }
}