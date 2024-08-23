mod actor;
mod engine;
mod error;
mod message;

pub use engine::DB;

pub mod prelude {
    pub use crate::actor::{ActorId, Post};
    pub use crate::dbg_export_db;
    pub use crate::engine::Engine;
    pub use crate::error::ActorError;
    pub use crate::message::{Message, MessageRx};
}
