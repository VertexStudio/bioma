use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BehaviorTick;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum BehaviorStatus {
    Running,
    Success,
    Failure,
}

pub trait Behavior: Clone + Debug + Serialize + for<'de> Deserialize<'de> {}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionBehavior<B: Behavior> {
    _marker: std::marker::PhantomData<B>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecoratorBehavior<B: Behavior> {
    child: ActorId,
    _marker: std::marker::PhantomData<B>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompositeBehavior<B: Behavior> {
    children: Vec<ActorId>,
    _marker: std::marker::PhantomData<B>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // MockAction behavior
    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct MockAction;

    impl Behavior for MockAction {}

    impl Message<BehaviorTick> for ActionBehavior<MockAction> {
        type Response = BehaviorStatus;

        fn handle(
            &mut self,
            _ctx: &mut ActorContext<Self>,
            _msg: &BehaviorTick,
        ) -> impl Future<Output = Result<BehaviorStatus, ActorError>> {
            async move { Ok(BehaviorStatus::Success) }
        }
    }

    impl Actor for ActionBehavior<MockAction> {
        fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), ActorError>> {
            async move {
                let mut stream = ctx.recv().await?;
                while let Some(Ok(frame)) = stream.next().await {
                    if let Some(BehaviorTick) = ctx.is::<Self, BehaviorTick>(&frame) {
                        self.reply(ctx, &BehaviorTick, &frame).await?;
                    }
                }
                Ok(())
            }
        }
    }

    // MockDecorator behavior
    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct MockDecorator;

    impl Behavior for MockDecorator {}

    impl Message<BehaviorTick> for DecoratorBehavior<MockDecorator> {
        type Response = BehaviorStatus;

        fn handle(
            &mut self,
            ctx: &mut ActorContext<Self>,
            _msg: &BehaviorTick,
        ) -> impl Future<Output = Result<BehaviorStatus, ActorError>> {
            async move {
                let status = ctx.send_as::<BehaviorTick, BehaviorStatus>(BehaviorTick, &self.child).await?;
                Ok(status)
            }
        }
    }

    impl Actor for DecoratorBehavior<MockDecorator> {
        fn start(&mut self, _ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), ActorError>> {
            async move { Ok(()) }
        }
    }

    #[test]
    fn test_action_behavior() {}
}
