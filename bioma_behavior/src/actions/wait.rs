use crate::prelude::*;
use bioma_actor::prelude::*;
use bon::Builder;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::debug;

/// Waits for a specified duration, then succeeds.
///
/// The `Wait` action pauses for the given duration when ticked and always returns success after the
/// delay period has elapsed.
#[derive(Builder, Debug, Serialize, Deserialize)]
pub struct Wait {
    #[serde(with = "humantime_serde")]
    pub duration: Duration,
    #[serde(skip)]
    #[builder(skip)]
    pub node: behavior::Action,
}

impl Behavior for Wait {
    fn node(&self) -> behavior::Node {
        behavior::Node::Action(&self.node)
    }
}

pub struct WaitFactory;

impl ActorFactory for WaitFactory {
    fn spawn(
        &self,
        engine: Engine,
        config: serde_json::Value,
        id: ActorId,
        options: SpawnOptions,
    ) -> Result<ActorHandle, SystemActorError> {
        let engine = engine.clone();
        let node: tree::ActionNode = serde_json::from_value(config.clone()).unwrap();
        let config: Wait = serde_json::from_value(node.data.config.clone())?;
        Ok(tokio::spawn(async move {
            let (mut ctx, mut actor) = Actor::spawn(engine, id, config, options).await?;
            debug!("WaitFactory::spawn: start {}", ctx.id());
            actor.start(&mut ctx).await?;
            debug!("WaitFactory::spawn: end {}", ctx.id());
            Ok(())
        }))
    }
}

impl Message<BehaviorTick> for Wait {
    type Response = BehaviorStatus;

    async fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        _msg: &BehaviorTick,
    ) -> Result<BehaviorStatus, Self::Error> {
        tokio::time::sleep(self.duration).await;
        Ok(BehaviorStatus::Success)
    }
}

impl Actor for Wait {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(BehaviorTick) = frame.is::<BehaviorTick>() {
                self.reply(ctx, &BehaviorTick, &frame).await?;
                break;
            }
        }
        Ok(())
    }
}
