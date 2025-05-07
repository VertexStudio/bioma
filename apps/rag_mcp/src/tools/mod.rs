pub mod delete;
pub mod embed;
pub mod generate;
pub mod index;
pub mod ingest;
pub mod rerank;
pub mod retrieve;
pub mod sources;

use anyhow::Result;
use bioma_actor::{Actor, ActorContext, ActorId, Engine, Relay, SpawnExistsOptions, SpawnOptions, SystemActorError};
use uuid::Uuid;

pub struct ToolRelay {
    pub ctx: ActorContext<Relay>,
}

impl ToolRelay {
    pub async fn new(engine: &Engine, prefix: &str) -> Result<Self, SystemActorError> {
        let uuid = Uuid::new_v4();
        let actor_id = ActorId::of::<Relay>(format!("{}/{}", prefix, uuid.to_string()));

        let (ctx, _) = Actor::spawn(
            engine.clone(),
            actor_id,
            Relay,
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;

        Ok(Self { ctx })
    }
}
