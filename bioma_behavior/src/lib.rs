mod behavior;
mod error;

pub mod actions;
pub mod composites;

pub mod prelude {
    pub use crate::actions;
    pub use crate::behavior::{Behavior, BehaviorNode, BehaviorType};
    pub use crate::composites;
    pub use crate::error::BehaviorError;
    pub use crate::impl_behavior_node;
}
