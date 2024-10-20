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
    pub use crate::tree::BehaviorTree;
    pub use bioma_actor::Message;
}

pub async fn register_behaviors(registry: &bioma_actor::ActorTagRegistry) -> Result<(), bioma_actor::SystemActorError> {
    // Actions
    registry.add("Wait", actions::WaitFactory).await?;
    registry.add("Log", actions::LogFactory).await?;

    // Decorators
    registry.add("Delay", decorators::DelayFactory).await?;

    // Composites
    registry.add("All", composites::AllFactory).await?;
    registry.add("Any", composites::AnyFactory).await?;
    registry.add("Fallback", composites::FallbackFactory).await?;
    registry.add("Sequence", composites::SequenceFactory).await?;
    Ok(())
}
