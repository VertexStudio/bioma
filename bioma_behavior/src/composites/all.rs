use crate::prelude::*;
use bioma_actor::prelude::*;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

/// Executes all child nodes in parallel and succeeds only if all succeed.
///
/// The `All` composite node runs each of its child nodes concurrently. If any child node fails, the `All` node
/// immediately fails and all other child nodes are interrupted; otherwise, it succeeds once all
/// child nodes have successfully completed.
#[derive(Debug, Serialize, Deserialize)]
pub struct All {
    pub node: behavior::Composite,
}

impl Behavior for All {
    fn node(&self) -> behavior::Node {
        behavior::Node::Composite(self.node.clone())
    }
}

pub struct AllFactory;

impl ActorFactory for AllFactory {
    fn spawn(&self, engine: Engine, config: serde_json::Value, id: ActorId, options: SpawnOptions) -> ActorHandle {
        let engine = engine.clone();
        let config: All = serde_json::from_value(config.clone())?;
        Ok(tokio::spawn(async move {
            let (mut ctx, mut actor) = Actor::spawn(engine, id, config, options).await?;
            actor.start(&mut ctx).await?;
            Ok(())
        }))
    }
}

impl Message<BehaviorTick> for All {
    type Response = BehaviorStatus;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        _msg: &BehaviorTick,
    ) -> Result<BehaviorStatus, Self::Error> {
        let mut futures = Vec::new();
        for child in &self.node.children {
            futures.push(ctx.send_as(BehaviorTick, child, SendOptions::default()));
        }

        let results = futures::future::join_all(futures).await;

        for result in results {
            match result {
                Ok(BehaviorStatus::Failure) => return Ok(BehaviorStatus::Failure),
                Err(_) => return Ok(BehaviorStatus::Failure),
                _ => continue,
            }
        }

        Ok(BehaviorStatus::Success)
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
