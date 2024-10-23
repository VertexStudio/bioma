use crate::prelude::*;
use bioma_actor::prelude::*;
use bon::Builder;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::timeout;
use tracing::debug;

/// Executes its child node with a timeout.
///
/// The `Timeout` decorator node attempts to execute its child node within a specified duration.
/// If the child node completes before the timeout, it returns the child's result.
/// If the timeout occurs first, it returns a failure status.
#[derive(Builder, Debug, Serialize, Deserialize)]
pub struct Timeout {
    #[serde(with = "humantime_serde")]
    pub duration: Duration,
    #[serde(skip)]
    #[builder(skip)]
    pub node: behavior::Decorator,
}

impl Behavior for Timeout {
    fn node(&self) -> behavior::Node {
        behavior::Node::Decorator(&self.node)
    }
}

pub struct TimeoutFactory;

impl ActorFactory for TimeoutFactory {
    fn spawn(
        &self,
        engine: Engine,
        config: serde_json::Value,
        id: ActorId,
        options: SpawnOptions,
    ) -> Result<ActorHandle, SystemActorError> {
        let node: crate::tree::DecoratorNode = serde_json::from_value(config.clone()).unwrap();
        let mut config: Timeout = serde_json::from_value(node.data.config.clone())?;
        config.node.copy_child(&node);
        Ok(tokio::spawn(async move {
            let (mut ctx, mut actor) = Actor::spawn(engine, id, config, options).await?;
            debug!("TimeoutFactory::spawn: start {}", ctx.id());
            actor.start(&mut ctx).await?;
            debug!("TimeoutFactory::spawn: end {}", ctx.id());
            Ok(())
        }))
    }
}

impl Message<BehaviorTick> for Timeout {
    type Response = BehaviorStatus;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        _msg: &BehaviorTick,
    ) -> Result<BehaviorStatus, Self::Error> {
        let Some(child) = self.node.child(ctx, SpawnOptions::default()).await? else {
            return Ok(BehaviorStatus::Success);
        };

        match timeout(self.duration, ctx.send_as(BehaviorTick, child.clone(), SendOptions::default())).await {
            Ok(Ok(status)) => Ok(status),
            Ok(Err(e)) => Err(e.into()),
            Err(_) => Ok(BehaviorStatus::Failure), // Timeout occurred
        }
    }
}

impl Actor for Timeout {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            self.reply(ctx, &BehaviorTick, &frame).await?;
        }
        Ok(())
    }
}
