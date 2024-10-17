pub mod behavior;
mod error;

pub mod actions;
pub mod composites;

pub mod prelude {
    pub use crate::actions;
    pub use crate::behavior::{
        ActionBehavior, Behavior, BehaviorStatus, BehaviorTick, CompositeBehavior, DecoratorBehavior,
    };
    pub use crate::composites;
    pub use crate::error::BehaviorError;
    pub use bioma_actor::Message;
}
