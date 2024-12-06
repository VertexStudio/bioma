pub mod chat;
pub mod embeddings;
pub mod indexer;
pub mod pdf_analyzer;
pub mod rerank;
pub mod retriever;

pub mod prelude {
    pub use crate::chat::{self, Chat, ChatError, ChatStreamChunk, ChatStreamNext, ChatStreamStart};
    pub use crate::embeddings::{
        self, EmbeddingContent, Embeddings, EmbeddingsError, GenerateEmbeddings, GeneratedEmbeddings, StoreEmbeddings,
    };
    pub use crate::indexer::{self, DeleteSource, DeletedSource, IndexGlobs, Indexer, IndexerError};
    pub use crate::rerank::{self, RankTexts, RankedText, RankedTexts, Rerank, RerankError};
    pub use crate::retriever::{self, RetrieveContext, RetrieveQuery, Retriever, RetrieverError};
    pub use ollama_rs::generation::chat::{ChatMessage, ChatMessageResponse, MessageRole};
}
