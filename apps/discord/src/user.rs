use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct UserActor {}

impl Actor for UserActor {
    type Error = SystemActorError;

    async fn start(&mut self, _ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        Ok(())
    }
}
