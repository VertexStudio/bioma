pub mod chat;
pub mod embeddings;
pub mod indexer;
pub mod markitdown;
pub mod pdf_analyzer;
pub mod rerank;
pub mod retriever;
pub mod summary;

pub mod prelude {
    pub use crate::chat::{self, Chat, ChatError, ChatMessages};
    pub use crate::embeddings::{
        self, EmbeddingContent, Embeddings, EmbeddingsError, GenerateEmbeddings, GeneratedEmbeddings, ImageData,
        StoreEmbeddings,
    };
    pub use crate::indexer::{
        self, DeleteSource, DeletedSource, GlobsContent, Index, IndexContent, Indexer, IndexerError, TextChunkConfig,
    };
    pub use crate::rerank::{self, RankTexts, RankedText, RankedTexts, Rerank, RerankError};
    pub use crate::retriever::{self, RetrieveContext, RetrieveQuery, Retriever, RetrieverError};
    pub use crate::summary::{self, Summarize, Summary, SummaryError, SummaryResponse};
    pub use ollama_rs::generation::{
        chat::{ChatMessage, ChatMessageResponse, MessageRole},
        images::Image,
    };
}
