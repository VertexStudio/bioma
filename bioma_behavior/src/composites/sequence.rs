use crate::prelude::*;
use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};

/// Executes child nodes sequentially until one fails or all succeed.
///
/// The `Sequence` composite node processes its children one by one in order. It returns success only if
/// all child nodes succeed. If a child fails, the `Sequence` node immediately fails. If a child
/// returns running, the `Sequence` node also returns running.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sequence;

impl Behavior for Sequence {}

impl Message<BehaviorTick> for CompositeBehavior<Sequence> {
    type Response = BehaviorStatus;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        _msg: &BehaviorTick,
    ) -> Result<BehaviorStatus, Self::Error> {
        for child in &self.children {
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

impl Actor for CompositeBehavior<Sequence> {
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
