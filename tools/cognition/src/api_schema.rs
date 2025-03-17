use actix_multipart::form::{json::Json as MpJson, tempfile::TempFile, MultipartForm};
use bioma_llm::{
    chat,
    indexer::{ImagesContent, TextsContent},
    prelude::{ChatMessage, GlobsContent, Index, IndexContent, RetrieveContext, RetrieveQuery, TextChunkConfig},
    retriever::{default_retriever_limit, default_retriever_sources, default_retriever_threshold},
};
use ollama_rs::{generation::tools::ToolInfo, models::ModelOptions};
use serde::{Deserialize, Serialize};

//------------------------------------------------------------------------------
// Chat Module Schemas
//------------------------------------------------------------------------------

/// Request schema for chat completion
#[derive(utoipa::ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ChatQuery {
    /// The conversation history as a list of messages
    pub messages: Vec<ChatMessage>,

    /// List of sources to search for relevant context
    #[schema(default = default_retriever_sources)]
    #[serde(default = "default_retriever_sources")]
    pub sources: Vec<String>,

    /// Optional schema for structured output format
    pub format: Option<chat::Schema>,

    /// List of available tools for the chat
    #[serde(default)]
    pub tools: Vec<ToolInfo>,

    /// List of tool actor identifiers
    #[serde(default)]
    pub tools_actors: Vec<String>,

    /// Whether to stream the response
    #[serde(default = "default_chat_stream")]
    pub stream: bool,

    /// Generation options
    pub options: Option<ModelOptions>,
}

fn default_chat_stream() -> bool {
    true
}

/// Request schema for think operation
#[derive(utoipa::ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ThinkQuery {
    /// The conversation history as a list of messages
    pub messages: Vec<ChatMessage>,

    /// List of sources to search for relevant context
    #[schema(default = default_retriever_sources)]
    #[serde(default = "default_retriever_sources")]
    pub sources: Vec<String>,

    /// Optional schema for structured output format
    pub format: Option<chat::Schema>,

    /// List of available tools for thinking
    #[serde(default)]
    pub tools: Vec<ToolInfo>,

    /// List of tool actor identifiers
    #[serde(default)]
    pub tools_actors: Vec<String>,

    /// Whether to stream the response
    #[serde(default = "default_think_stream")]
    pub stream: bool,

    /// Generation options
    pub options: Option<ModelOptions>,
}

fn default_think_stream() -> bool {
    true
}

/// Request schema for asking a question
#[derive(utoipa::ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct AskQuery {
    /// The conversation history as a list of messages
    pub messages: Vec<ChatMessage>,

    /// List of sources to search for relevant context
    #[schema(default = default_retriever_sources)]
    #[serde(default = "default_retriever_sources")]
    pub sources: Vec<String>,

    /// Optional schema for structured output format
    pub format: Option<chat::Schema>,

    /// Generation options
    pub options: Option<ModelOptions>,
}

//------------------------------------------------------------------------------
// Embeddings Module Schemas
//------------------------------------------------------------------------------

/// Available embedding models
#[derive(utoipa::ToSchema, Deserialize)]
pub enum ModelEmbed {
    #[serde(rename = "nomic-embed-text")]
    NomicEmbedTextV15,
    #[serde(rename = "nomic-embed-vision")]
    NomicEmbedVisionV15,
}

/// Request schema for generating embeddings
#[derive(utoipa::ToSchema, Deserialize)]
pub struct EmbeddingsQuery {
    /// The embedding model to use
    pub model: ModelEmbed,

    /// The input data to generate embeddings for (text or base64-encoded image)
    pub input: serde_json::Value,
}

//------------------------------------------------------------------------------
// Indexer Module Schemas
//------------------------------------------------------------------------------

/// Request schema for indexing content
#[derive(utoipa::ToSchema, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IndexRequest {
    /// Index content using glob patterns
    Globs(IndexGlobs),
    /// Index text content directly
    Texts(IndexTexts),
    /// Index image content
    Images(IndexImages),
}

/// Request schema for indexing files using glob patterns
#[derive(utoipa::ToSchema, Clone, Serialize, Deserialize)]
pub struct IndexGlobs {
    /// The glob patterns to match files for indexing
    #[serde(flatten)]
    pub content: GlobsContent,

    /// The source identifier for the indexed content
    #[schema(default = default_source)]
    #[serde(default = "default_source")]
    pub source: String,

    /// Whether to summarize each file
    #[schema(default = false)]
    #[serde(default)]
    pub summarize: bool,
}

/// Request schema for indexing text content directly
#[derive(utoipa::ToSchema, Clone, Serialize, Deserialize)]
pub struct IndexTexts {
    /// The text content to index
    #[serde(flatten)]
    pub content: TextsContent,

    /// The source identifier for the indexed content
    #[schema(default = default_source)]
    #[serde(default = "default_source")]
    pub source: String,

    /// Whether to summarize each text
    #[schema(default = false)]
    #[serde(default)]
    pub summarize: bool,
}

/// Request schema for indexing image content
#[derive(utoipa::ToSchema, Clone, Serialize, Deserialize)]
pub struct IndexImages {
    /// The image content to index
    #[serde(flatten)]
    pub content: ImagesContent,

    /// The source identifier for the indexed content
    #[schema(default = default_source)]
    #[serde(default = "default_source")]
    pub source: String,

    /// Whether to summarize each image
    #[schema(default = false)]
    #[serde(default)]
    pub summarize: bool,
}

fn default_source() -> String {
    "/global".to_string()
}

impl From<IndexRequest> for Index {
    fn from(val: IndexRequest) -> Self {
        match val {
            IndexRequest::Globs(globs) => {
                let chunk_capacity = std::ops::Range {
                    start: globs.content.config.chunk_capacity.start,
                    end: globs.content.config.chunk_capacity.end,
                };

                Index::builder()
                    .source(globs.source)
                    .content(IndexContent::Globs(
                        GlobsContent::builder()
                            .globs(globs.content.globs)
                            .config(
                                TextChunkConfig::builder()
                                    .chunk_capacity(chunk_capacity)
                                    .chunk_overlap(globs.content.config.chunk_overlap)
                                    .chunk_batch_size(globs.content.config.chunk_batch_size)
                                    .build(),
                            )
                            .build(),
                    ))
                    .summarize(globs.summarize)
                    .build()
            }
            IndexRequest::Texts(texts) => {
                let chunk_capacity = std::ops::Range {
                    start: texts.content.config.chunk_capacity.start,
                    end: texts.content.config.chunk_capacity.end,
                };

                Index::builder()
                    .source(texts.source)
                    .content(IndexContent::Texts(
                        TextsContent::builder()
                            .texts(texts.content.texts)
                            .mime_type(texts.content.mime_type)
                            .config(
                                TextChunkConfig::builder()
                                    .chunk_capacity(chunk_capacity)
                                    .chunk_overlap(texts.content.config.chunk_overlap)
                                    .chunk_batch_size(texts.content.config.chunk_batch_size)
                                    .build(),
                            )
                            .build(),
                    ))
                    .summarize(texts.summarize)
                    .build()
            }
            IndexRequest::Images(images) => {
                let content = ImagesContent { images: images.content.images, mime_type: images.content.mime_type };

                Index::builder()
                    .source(images.source)
                    .content(IndexContent::Images(content))
                    .summarize(images.summarize)
                    .build()
            }
        }
    }
}

//------------------------------------------------------------------------------
// Retriever Module Schemas
//------------------------------------------------------------------------------

/// Output format for retrieval results
#[derive(utoipa::ToSchema, Debug, Clone, Serialize, Deserialize)]
pub enum RetrieveOutputFormat {
    #[serde(rename = "markdown")]
    Markdown,
    #[serde(rename = "json")]
    Json,
}

/// Request schema for retrieving context
#[derive(utoipa::ToSchema, Debug, Clone, Serialize, Deserialize)]
#[schema(example = json!({
    "type": "Text",
    "query": "What is Bioma?",
    "threshold": 0.0,
    "limit": 10,
    "sources": ["path/to/source1", "path/to/source2"],
    "format": "markdown"
}), as = RetrieveContext)]
pub struct RetrieveContextRequest {
    /// The type of query
    pub r#type: RetrieveType,

    /// The query text
    pub query: String,

    /// The number of contexts to return
    #[schema(default = default_retriever_limit)]
    #[serde(default = "default_retriever_limit")]
    pub limit: usize,

    /// The threshold for the similarity score
    #[schema(default = default_retriever_threshold)]
    #[serde(default = "default_retriever_threshold")]
    pub threshold: f32,

    /// A list of sources to filter the search
    #[schema(default = default_retriever_sources)]
    #[serde(default = "default_retriever_sources")]
    pub sources: Vec<String>,

    /// The format of the output (markdown or json)
    #[schema(default = default_retriever_format)]
    #[serde(default = "default_retriever_format")]
    pub format: RetrieveOutputFormat,
}

#[derive(utoipa::ToSchema, Debug, Clone, Serialize, Deserialize)]
pub enum RetrieveType {
    Text,
}

fn default_retriever_format() -> RetrieveOutputFormat {
    RetrieveOutputFormat::Markdown
}

impl From<RetrieveContextRequest> for RetrieveContext {
    fn from(request: RetrieveContextRequest) -> Self {
        RetrieveContext::builder()
            .query(RetrieveQuery::Text(request.query))
            .threshold(request.threshold)
            .limit(request.limit)
            .sources(request.sources)
            .build()
    }
}

//------------------------------------------------------------------------------
// File Operations Schemas
//------------------------------------------------------------------------------

/// Metadata for file upload
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UploadMetadata {
    /// Target path where the file should be stored
    #[schema(value_type = String)]
    pub path: std::path::PathBuf,
}

/// Request schema for file upload
#[derive(utoipa::ToSchema, Debug, MultipartForm)]
pub struct Upload {
    /// The file to upload (max size: 100MB)
    #[multipart(limit = "100MB")]
    #[schema(value_type = String, format = Binary)]
    pub file: TempFile,

    /// Metadata about the upload
    #[multipart(rename = "metadata")]
    #[schema(value_type = UploadMetadata)]
    pub metadata: MpJson<UploadMetadata>,
}
