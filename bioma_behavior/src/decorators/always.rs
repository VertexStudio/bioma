use crate::prelude::*;
use bioma_actor::prelude::*;
use bon::Builder;
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Always returns a specified status, regardless of its child node's result.
///
/// The `Always` decorator node executes its child node but always returns the
/// configured status (Success or Failure), ignoring the child's result.
#[derive(Builder, Debug, Serialize, Deserialize)]
pub struct Always {
    pub success: bool,
    #[serde(skip)]
    #[builder(skip)]
    pub node: behavior::Decorator,
}

impl Behavior for Always {
    fn node(&self) -> behavior::Node {
        behavior::Node::Decorator(&self.node)
    }
}

pub struct AlwaysFactory;

impl ActorFactory for AlwaysFactory {
    fn spawn(
        &self,
        engine: Engine,
        config: serde_json::Value,
        id: ActorId,
        options: SpawnOptions,
    ) -> Result<ActorHandle, SystemActorError> {
        let node: crate::tree::DecoratorNode = serde_json::from_value(config.clone()).unwrap();
        let mut config: Always = serde_json::from_value(node.data.config.clone())?;
        config.node.copy_child(&node);
        Ok(tokio::spawn(async move {
            let (mut ctx, mut actor) = Actor::spawn(engine, id, config, options).await?;
            debug!("AlwaysFactory::spawn: start {}", ctx.id());
            actor.start(&mut ctx).await?;
            debug!("AlwaysFactory::spawn: end {}", ctx.id());
            Ok(())
        }))
    }
}

impl Message<BehaviorTick> for Always {
    type Response = BehaviorStatus;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, _msg: &BehaviorTick) -> Result<(), Self::Error> {
        let Some(child) = self.node.child(ctx, SpawnOptions::default()).await? else {
            ctx.reply(self.get_configured_status()).await?;
            return Ok(());
        };
        // Execute the child node but ignore its result
        let _status: Result<BehaviorStatus, SystemActorError> =
            ctx.send_as_and_wait_reply(BehaviorTick, child.clone(), SendOptions::default()).await;
        // Return the configured status
        ctx.reply(self.get_configured_status()).await?;
        Ok(())
    }
}

impl Actor for Always {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            self.reply(ctx, &BehaviorTick, &frame).await?;
        }
        Ok(())
    }
}

impl Always {
    fn get_configured_status(&self) -> BehaviorStatus {
        if self.success {
            BehaviorStatus::Success
        } else {
            BehaviorStatus::Failure
        }
    }
}
