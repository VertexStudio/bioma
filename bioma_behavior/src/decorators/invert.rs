use crate::prelude::*;
use bioma_actor::prelude::*;
use bon::Builder;
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Inverts the result of its child node.
///
/// The `Invert` decorator node executes its child node and then inverts the result:
/// Success becomes Failure, Failure becomes Success, and Running remains unchanged.
#[derive(Builder, Debug, Serialize, Deserialize)]
pub struct Invert {
    #[serde(skip)]
    #[builder(skip)]
    pub node: behavior::Decorator,
}

impl Behavior for Invert {
    fn node(&self) -> behavior::Node {
        behavior::Node::Decorator(&self.node)
    }
}

pub struct InvertFactory;

impl ActorFactory for InvertFactory {
    fn spawn(
        &self,
        engine: Engine,
        config: serde_json::Value,
        id: ActorId,
        options: SpawnOptions,
    ) -> Result<ActorHandle, SystemActorError> {
        let node: crate::tree::DecoratorNode = serde_json::from_value(config.clone()).unwrap();
        let mut config: Invert = serde_json::from_value(node.data.config.clone())?;
        config.node.copy_child(&node);
        Ok(tokio::spawn(async move {
            let (mut ctx, mut actor) = Actor::spawn(engine, id, config, options).await?;
            debug!("InvertFactory::spawn: start {}", ctx.id());
            actor.start(&mut ctx).await?;
            debug!("InvertFactory::spawn: end {}", ctx.id());
            Ok(())
        }))
    }
}

impl Message<BehaviorTick> for Invert {
    type Response = BehaviorStatus;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, _msg: &BehaviorTick) -> Result<(), Self::Error> {
        let Some(child) = self.node.child(ctx, SpawnOptions::default()).await? else {
            ctx.reply(BehaviorStatus::Failure).await?;
            return Ok(());
        };

        // Execute the child node and invert its result
        let status = match ctx
            .send_as_and_wait_reply::<BehaviorTick, BehaviorStatus>(BehaviorTick, child.clone(), SendOptions::default())
            .await
        {
            Ok(BehaviorStatus::Success) => BehaviorStatus::Failure,
            Ok(BehaviorStatus::Failure) => BehaviorStatus::Success,
            Err(_) => BehaviorStatus::Failure,
        };

        ctx.reply(status).await?;
        Ok(())
    }
}

impl Actor for Invert {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            self.reply(ctx, &BehaviorTick, &frame).await?;
        }
        Ok(())
    }
}
