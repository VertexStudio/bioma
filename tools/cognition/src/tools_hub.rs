use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolsHubActor {}

impl Actor for ToolsHubActor {
    type Error = SystemActorError;

    async fn start(&mut self, _ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ToolsHubActor {
    pub async fn new(
        engine: &Engine,
        tools_actor_id: Option<String>,
    ) -> Result<ActorContext<ToolsHubActor>, SystemActorError> {
        let actor_id = match tools_actor_id {
            Some(id) => format!("{}", id),
            None => {
                let ulid = ulid::Ulid::new();
                format!("{}{}", "/rag/server/tools_hub/", ulid.to_string())
            }
        };

        let actor_id = ActorId::of::<ToolsHubActor>(actor_id);
        let user_actor = ToolsHubActor {};
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
