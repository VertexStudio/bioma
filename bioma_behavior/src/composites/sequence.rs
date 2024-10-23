use crate::prelude::*;
use bioma_actor::prelude::*;
use bon::Builder;
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Executes child nodes sequentially until one fails or all succeed.
///
/// The `Sequence` composite node processes its children one by one in order. It returns success only if
/// all child nodes succeed. If a child fails, the `Sequence` node immediately fails. If a child
/// returns running, the `Sequence` node also returns running.
#[derive(Builder, Debug, Serialize, Deserialize)]
pub struct Sequence {
    #[serde(skip)]
    #[builder(skip)]
    pub node: behavior::Composite,
}

impl Behavior for Sequence {
    fn node(&self) -> behavior::Node {
        behavior::Node::Composite(&self.node)
    }
}

pub struct SequenceFactory;

impl ActorFactory for SequenceFactory {
    fn spawn(
        &self,
        engine: Engine,
        config: serde_json::Value,
        id: ActorId,
        options: SpawnOptions,
    ) -> Result<ActorHandle, SystemActorError> {
        let engine = engine.clone();
        let node: tree::CompositeNode = serde_json::from_value(config.clone()).unwrap();
        let mut config: Sequence = serde_json::from_value(node.data.config.clone())?;
        config.node.copy_children(&node);
        Ok(tokio::spawn(async move {
            let (mut ctx, mut actor) = Actor::spawn(engine, id, config, options).await?;
            debug!("SequenceFactory::spawn: start {}", ctx.id());
            actor.start(&mut ctx).await?;
            debug!("SequenceFactory::spawn: end {}", ctx.id());
            Ok(())
        }))
    }
}

impl Message<BehaviorTick> for Sequence {
    type Response = BehaviorStatus;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        _msg: &BehaviorTick,
    ) -> Result<BehaviorStatus, Self::Error> {
        // Iterate over all children until one fails
        for child in self.node.children(ctx, SpawnOptions::default()).await? {
            let status = ctx.send_as(BehaviorTick, child, SendOptions::default()).await;
            match status {
                Ok(BehaviorStatus::Success) => continue,
                Ok(BehaviorStatus::Failure) => return Ok(BehaviorStatus::Failure),
                Err(_e) => return Ok(BehaviorStatus::Failure),
            }
        }
        Ok(BehaviorStatus::Success)
    }
}

impl Actor for Sequence {
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
