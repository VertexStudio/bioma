use crate::prelude::*;
use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};

/// Executes child nodes sequentially until one succeeds or all fail.
///
/// The `Fallback` composite node processes its children one by one in order. It returns success as soon as one
/// child node succeeds. If a child fails, it proceeds to the next one. If all children fail,
/// then the `Fallback` node fails.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fallback {
    pub node: behavior::Composite,
}

impl Behavior for Fallback {
    fn node(&self) -> behavior::Node {
        behavior::Node::Composite(&self.node)
    }
}

impl Message<BehaviorTick> for Fallback {
    type Response = BehaviorStatus;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        _msg: &BehaviorTick,
    ) -> Result<BehaviorStatus, Self::Error> {
        for child in &self.node.children {
            let status = ctx.send_as(BehaviorTick, child, SendOptions::default()).await;
            match status {
                Ok(BehaviorStatus::Success) => return Ok(BehaviorStatus::Success),
                Ok(BehaviorStatus::Failure) => continue,
                Err(_e) => continue,
            }
        }
        Ok(BehaviorStatus::Failure)
    }
}

impl Actor for Fallback {
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
