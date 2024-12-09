use crate::prelude::*;
use bioma_actor::prelude::*;
use bon::Builder;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Executes all child nodes in parallel and succeeds only if all succeed.
///
/// The `All` composite node runs each of its child nodes concurrently. If any child node fails, the `All` node
/// immediately fails and all other child nodes are interrupted; otherwise, it succeeds once all
/// child nodes have successfully completed.
#[derive(Builder, Debug, Serialize, Deserialize)]
pub struct All {
    #[serde(skip)]
    #[builder(skip)]
    pub node: behavior::Composite,
}

impl Behavior for All {
    fn node(&self) -> behavior::Node {
        behavior::Node::Composite(&self.node)
    }
}

pub struct AllFactory;

impl ActorFactory for AllFactory {
    fn spawn(
        &self,
        engine: Engine,
        config: serde_json::Value,
        id: ActorId,
        options: SpawnOptions,
    ) -> Result<ActorHandle, SystemActorError> {
        let engine = engine.clone();
        let node: tree::CompositeNode = serde_json::from_value(config.clone()).unwrap();
        let mut config: All = serde_json::from_value(node.data.config.clone())?;
        config.node.copy_children(&node);
        Ok(tokio::spawn(async move {
            let (mut ctx, mut actor) = Actor::spawn(engine, id, config, options).await?;
            debug!("AllFactory::spawn: start {}", ctx.id());
            actor.start(&mut ctx).await?;
            debug!("AllFactory::spawn: end {}", ctx.id());
            Ok(())
        }))
    }
}

impl Message<BehaviorTick> for All {
    type Response = BehaviorStatus;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, _msg: &BehaviorTick) -> Result<(), Self::Error> {
        let children = self.node.children(ctx, SpawnOptions::default()).await?;

        // Create a future for each child and pin it
        let futures = children
            .iter()
            .map(|child| Box::pin(ctx.send_as_and_wait_reply(BehaviorTick, child.clone(), SendOptions::default())))
            .collect::<Vec<_>>();

        // Use futures::future::select_all to run all futures concurrently
        let mut remaining_futures = futures;
        let mut overall_status = BehaviorStatus::Success;

        // Wait for all futures to complete
        while !remaining_futures.is_empty() {
            let (result, _index, remaining) = futures::future::select_all(remaining_futures).await;
            remaining_futures = remaining;

            match result {
                Ok(BehaviorStatus::Failure) => {
                    overall_status = BehaviorStatus::Failure;
                    break;
                }
                Err(_) => {
                    overall_status = BehaviorStatus::Failure;
                    break;
                }
                _ => continue,
            }
        }

        if overall_status == BehaviorStatus::Failure {
            // Stop all children
            for child_idx in 0..children.len() {
                self.node.child_stop(child_idx);
            }
        }

        ctx.reply(overall_status).await?;
        Ok(())
    }
}

impl Actor for All {
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
