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
        behavior::Node::Composite(self.node.clone())
    }
}

pub struct AnyFactory;

impl ActorFactory for AnyFactory {
    fn spawn(&self, engine: Engine, config: serde_json::Value, id: ActorId, options: SpawnOptions) -> ActorHandle {
        let engine = engine.clone();
        let config: Any = serde_json::from_value(config.clone())?;
        Ok(tokio::spawn(async move {
            let (mut ctx, mut actor) = Actor::spawn(engine, id, config, options).await?;
            actor.start(&mut ctx).await?;
            Ok(())
        }))
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
