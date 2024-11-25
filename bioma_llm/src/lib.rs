pub mod chat;
pub mod embeddings;
pub mod indexer;
pub mod pdf_analyzer;
pub mod rerank;
pub mod retriever;

pub mod prelude {
    pub use crate::chat::{self, Chat, ChatError, ChatMessages};
    pub use crate::embeddings::{
        self, Embeddings, EmbeddingsError, GenerateTextEmbeddings, GeneratedTextEmbeddings, StoreTextEmbeddings,
        DEFAULT_IMAGE_EMBEDDING_LENGTH, DEFAULT_TEXT_EMBEDDING_LENGTH,
    };
    pub use crate::indexer::{self, DeleteSource, DeletedSource, IndexGlobs, Indexer, IndexerError};
    pub use crate::rerank::{self, RankTexts, RankedText, RankedTexts, Rerank, RerankError};
    pub use crate::retriever::{self, RetrieveContext, Retriever, RetrieverError};
    pub use ollama_rs::generation::chat::{ChatMessage, ChatMessageResponse, MessageRole};
}
