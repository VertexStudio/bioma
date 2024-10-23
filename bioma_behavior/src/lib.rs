pub mod behavior;
mod error;
pub mod tree;

pub mod actions;
pub mod composites;
pub mod decorators;

pub mod prelude {
    pub use crate::actions;
    pub use crate::behavior::{self, Behavior, BehaviorCancel, BehaviorStatus, BehaviorTick};
    pub use crate::composites;
    pub use crate::decorators;
    pub use crate::error::BehaviorError;
    pub use crate::tree::{self, BehaviorTree};
    pub use bioma_actor::Message;
}

pub async fn register_behaviors(registry: &bioma_actor::ActorTagRegistry) -> Result<(), bioma_actor::SystemActorError> {
    use crate::behavior::Behavior;

    // Actions
    registry.add(actions::Wait::tag(), actions::WaitFactory).await?;
    registry.add(actions::Log::tag(), actions::LogFactory).await?;

    // Decorators
    registry.add(decorators::Delay::tag(), decorators::DelayFactory).await?;

    // Composites
    registry.add(composites::All::tag(), composites::AllFactory).await?;
    registry.add(composites::Any::tag(), composites::AnyFactory).await?;
    registry.add(composites::Fallback::tag(), composites::FallbackFactory).await?;
    registry.add(composites::Sequence::tag(), composites::SequenceFactory).await?;
    Ok(())
}
