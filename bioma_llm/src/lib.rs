pub mod chat;
pub mod embeddings;
pub mod indexer;
pub mod pdf_analyzer;
pub mod rerank;
pub mod retriever;

pub mod prelude {
    pub use crate::chat::{self, Chat, ChatError, ChatMessage, ChatRequest};
    pub use crate::embeddings::{
        self, Embeddings, EmbeddingsError, GenerateTextEmbeddings, StoreTextEmbeddings, DEFAULT_EMBEDDING_LENGTH,
    };
    pub use crate::indexer::{self, DeleteSource, DeletedSource, IndexGlobs, Indexer, IndexerError};
    pub use crate::rerank::{self, RankTexts, RankedText, RankedTexts, Rerank, RerankError};
    pub use crate::retriever::{self, RetrieveContext, Retriever, RetrieverError};
    pub use mistralrs::{ChatCompletionResponse, TextMessageRole};
}
