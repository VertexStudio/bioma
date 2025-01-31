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

impl UserActor {
    pub async fn new(
        engine: &Engine,
        tools_actor_id: Option<String>,
    ) -> Result<ActorContext<UserActor>, SystemActorError> {
        let actor_id = match tools_actor_id {
            Some(id) => format!("{}", id),
            None => {
                let ulid = ulid::Ulid::new();
                format!("{}{}", "/rag/client/tool/", ulid.to_string())
            }
        };

        let actor_id = ActorId::of::<UserActor>(actor_id);
        let user_actor = UserActor {};
        let (ctx, _) = Actor::spawn(
            engine.clone(),
            actor_id,
            user_actor,
            SpawnOptions::builder().exists(SpawnExistsOptions::Restore).build(),
        )
        .await?;
        Ok(ctx)
    }
}
