use crate::prelude::*;
use bioma_actor::prelude::*;
use bon::Builder;
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Executes child nodes sequentially until one succeeds or all fail.
///
/// The `Fallback` composite node processes its children one by one in order. It returns success as soon as one
/// child node succeeds. If a child fails, it proceeds to the next one. If all children fail,
/// then the `Fallback` node fails.
#[derive(Builder, Debug, Serialize, Deserialize)]
pub struct Fallback {
    #[serde(skip)]
    #[builder(skip)]
    pub node: behavior::Composite,
}

impl Behavior for Fallback {
    fn node(&self) -> behavior::Node {
        behavior::Node::Composite(&self.node)
    }
}

pub struct FallbackFactory;

impl ActorFactory for FallbackFactory {
    fn spawn(
        &self,
        engine: Engine,
        config: serde_json::Value,
        id: ActorId,
        options: SpawnOptions,
    ) -> Result<ActorHandle, SystemActorError> {
        let engine = engine.clone();
        let node: tree::CompositeNode = serde_json::from_value(config.clone()).unwrap();
        let mut config: Fallback = serde_json::from_value(node.data.config.clone())?;
        config.node.copy_children(&node);
        Ok(tokio::spawn(async move {
            let (mut ctx, mut actor) = Actor::spawn(engine, id, config, options).await?;
            debug!("FallbackFactory::spawn: start {}", ctx.id());
            actor.start(&mut ctx).await?;
            debug!("FallbackFactory::spawn: end {}", ctx.id());
            Ok(())
        }))
    }
}

impl Message<BehaviorTick> for Fallback {
    type Response = BehaviorStatus;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        _msg: &BehaviorTick,
    ) -> Result<BehaviorStatus, Self::Error> {
        // Iterate over all children until one succeeds
        for child in self.node.children(ctx, SpawnOptions::default()).await? {
            let status = ctx.send_as(BehaviorTick, child, SendOptions::default()).await;
            match status {
                Ok(BehaviorStatus::Success) => return Ok(BehaviorStatus::Success),
                Ok(BehaviorStatus::Failure) => continue,
                Err(_e) => continue,
            }
        }
        Ok(BehaviorStatus::Failure)
    }
}

impl Actor for Fallback {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(BehaviorTick) = frame.is::<BehaviorTick>() {
                self.reply(ctx, &BehaviorTick, &frame).await?;
            }
        }
        Ok(())
    }
}
