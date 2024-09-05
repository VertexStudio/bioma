pub mod chat;
pub mod rerank;

pub mod prelude {
    pub use crate::chat::{self, Chat};
    pub use ollama_rs::generation::chat::{ChatMessage, ChatMessageResponse};
    pub use crate::rerank::{self, Rerank, RankTexts, RankedText};
}
