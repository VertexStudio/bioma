pub mod behavior;
mod error;

pub mod actions;
pub mod composites;

pub mod prelude {
    pub use crate::actions;
    pub use crate::behavior::{
        self, Behavior, BehaviorStatus, BehaviorTick,
    };
    pub use crate::composites;
    pub use crate::error::BehaviorError;
    pub use bioma_actor::Message;
}
