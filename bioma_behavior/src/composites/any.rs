use crate::prelude::*;
use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};

/// Executes all child nodes in parallel and succeeds if any one of them succeeds.
///
/// The `Any` composite node runs each of its child nodes concurrently. If any child node succeeds, the `Any` node
/// immediately succeeds and interrupts all other running child nodes; if all child nodes fail,
/// then the `Any` node fails.
#[derive(Debug, Serialize, Deserialize)]
pub struct Any {
    pub node: behavior::Composite,
}

impl Behavior for Any {
    fn node(&self) -> behavior::Node {
        behavior::Node::Composite(&self.node)
    }
}

impl Message<BehaviorTick> for Any {
    type Response = BehaviorStatus;

    async fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        _msg: &BehaviorTick,
    ) -> Result<BehaviorStatus, Self::Error> {
        Ok(BehaviorStatus::Success)
    }
}

impl Actor for Any {
    type Error = BehaviorError;

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
