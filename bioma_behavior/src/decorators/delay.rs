use crate::prelude::*;
use bioma_actor::prelude::*;
use bon::Builder;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::debug;

/// Delays execution before proceeding with its child node.
///
/// The `Delay` decorator node pauses for a specified duration before executing its child node. It returns the result
/// of the child node's execution.
#[derive(Builder, Debug, Serialize, Deserialize)]
pub struct Delay {
    #[serde(with = "humantime_serde")]
    pub duration: Duration,
    #[serde(skip)]
    #[builder(skip)]
    pub node: behavior::Decorator,
}

impl Behavior for Delay {
    fn node(&self) -> behavior::Node {
        behavior::Node::Decorator(&self.node)
    }
}

pub struct DelayFactory;

impl ActorFactory for DelayFactory {
    fn spawn(
        &self,
        engine: Engine,
        config: serde_json::Value,
        id: ActorId,
        options: SpawnOptions,
    ) -> Result<ActorHandle, SystemActorError> {
        let node: crate::tree::DecoratorNode = serde_json::from_value(config.clone()).unwrap();
        let mut config: Delay = serde_json::from_value(node.data.config.clone())?;
        config.node.copy_child(&node);
        Ok(tokio::spawn(async move {
            let (mut ctx, mut actor) = Actor::spawn(engine, id, config, options).await?;
            debug!("DelayFactory::spawn: start {}", ctx.id());
            actor.start(&mut ctx).await?;
            debug!("DelayFactory::spawn: end {}", ctx.id());
            Ok(())
        }))
    }
}

impl Message<BehaviorTick> for Delay {
    type Response = BehaviorStatus;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        _msg: &BehaviorTick,
    ) -> Result<BehaviorStatus, Self::Error> {
        tokio::time::sleep(self.duration).await;
        let Some(child) = self.node.child(ctx, SpawnOptions::default()).await? else {
            return Ok(BehaviorStatus::Success);
        };
        let status = ctx.send_as(BehaviorTick, child.clone(), SendOptions::default()).await;
        match status {
            Ok(status) => Ok(status),
            Err(e) => Err(e.into()),
        }
    }
}

impl Actor for Delay {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            self.reply(ctx, &BehaviorTick, &frame).await?;
        }
        Ok(())
    }
}
