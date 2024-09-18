pub mod chat;
pub mod embeddings;
pub mod rerank;
pub mod indexer;

pub mod prelude {
    pub use crate::chat::{self, Chat, ChatError};
    pub use crate::embeddings::{self, Embeddings, EmbeddingsError, GenerateEmbeddings};
    pub use crate::rerank::{self, RankTexts, RankedText, Rerank, RerankError};
    pub use ollama_rs::generation::chat::{ChatMessage, ChatMessageResponse};
}
