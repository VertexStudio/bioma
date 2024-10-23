use crate::prelude::*;
use bioma_actor::prelude::*;
use bon::Builder;
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Executes all child nodes in parallel and succeeds if any one of them succeeds.
///
/// The `Any` composite node runs each of its child nodes concurrently. If any child node succeeds, the `Any` node
/// immediately succeeds and interrupts all other running child nodes; if all child nodes fail,
/// then the `Any` node fails.
#[derive(Builder, Debug, Serialize, Deserialize)]
pub struct Any {
    #[serde(skip)]
    #[builder(skip)]
    pub node: behavior::Composite,
}

impl Behavior for Any {
    fn node(&self) -> behavior::Node {
        behavior::Node::Composite(&self.node)
    }
}

pub struct AnyFactory;

impl ActorFactory for AnyFactory {
    fn spawn(
        &self,
        engine: Engine,
        config: serde_json::Value,
        id: ActorId,
        options: SpawnOptions,
    ) -> Result<ActorHandle, SystemActorError> {
        let engine = engine.clone();
        let node: tree::CompositeNode = serde_json::from_value(config.clone()).unwrap();
        let mut config: Any = serde_json::from_value(node.data.config.clone())?;
        config.node.copy_children(&node);
        Ok(tokio::spawn(async move {
            let (mut ctx, mut actor) = Actor::spawn(engine, id, config, options).await?;
            debug!("AnyFactory::spawn: start {}", ctx.id());
            actor.start(&mut ctx).await?;
            debug!("AnyFactory::spawn: end {}", ctx.id());
            Ok(())
        }))
    }
}

impl Message<BehaviorTick> for Any {
    type Response = BehaviorStatus;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        _msg: &BehaviorTick,
    ) -> Result<BehaviorStatus, Self::Error> {
        let children = self.node.children(ctx, SpawnOptions::default()).await?;

        // Create a future for each child and pin it
        let futures = children
            .iter()
            .map(|child| Box::pin(ctx.send_as(BehaviorTick, child.clone(), SendOptions::default())))
            .collect::<Vec<_>>();

        // Use futures::future::select_all to run all futures concurrently
        let mut remaining_futures = futures;
        let mut overall_status = BehaviorStatus::Failure;

        // Wait for any future to complete
        while !remaining_futures.is_empty() {
            let (result, _index, remaining) = futures::future::select_all(remaining_futures).await;
            remaining_futures = remaining;

            match result {
                Ok(BehaviorStatus::Success) => {
                    overall_status = BehaviorStatus::Success;
                    break;
                }
                Err(_) => continue,
                _ => continue,
            }
        }

        if overall_status == BehaviorStatus::Success {
            // Stop all children
            for child_idx in 0..children.len() {
                self.node.child_stop(child_idx);
            }
        }

        Ok(overall_status)
    }
}

impl Actor for Any {
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
