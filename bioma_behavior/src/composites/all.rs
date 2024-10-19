use crate::prelude::*;
use bioma_actor::prelude::*;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

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
        behavior::Node::Composite(&self.node)
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
    type Error = BehaviorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        let (tx, mut rx) = mpsc::channel(100); // Create a channel for the message queue

        let mut stream = ctx.recv().await?;

        loop {
            tokio::select! {
                Some(Ok(frame)) = stream.next() => {
                    if let Some(BehaviorCancel) = frame.is::<BehaviorCancel>() {
                        // Empty the queue and break the loop if BehaviorCancel is received
                        while rx.try_recv().is_ok() {}
                        break;
                    } else {
                        // Add other messages to the queue
                        tx.send(frame).await.map_err(|_| BehaviorError::MessageQueue)?;
                    }
                }
                Some(frame) = rx.recv() => {
                    if let Some(BehaviorTick) = frame.is::<BehaviorTick>() {
                        // Process BehaviorTick messages
                        self.reply(ctx, &BehaviorTick, &frame).await?;
                    }
                }
                else => break,
            }
        }

        Ok(())
    }
}
