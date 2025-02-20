use actix_multipart::form::{json::Json as MpJson, tempfile::TempFile, MultipartForm};
use bioma_llm::{
    chat,
    indexer::{ImagesContent, TextsContent},
    prelude::{ChatMessage, GlobsContent, Index, IndexContent, RetrieveContext, TextChunkConfig},
    retriever::default_retriever_sources,
};
use ollama_rs::generation::tools::ToolInfo;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

//------------------------------------------------------------------------------
// Chat Module Schemas
//------------------------------------------------------------------------------

/// Request schema for chat completion
#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ChatQueryRequest {
    /// The conversation history as a list of messages
    pub messages: Vec<ChatMessage>,

    /// List of sources to search for relevant context
    #[schema(default = default_retriever_sources)]
    #[serde(default = "default_retriever_sources")]
    pub sources: Vec<String>,

    /// Optional schema for structured output format
    #[schema(value_type = Option<Schema::Object>)]
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
}

fn default_chat_stream() -> bool {
    true
}

/// Request schema for think operation
#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ThinkQueryRequest {
    /// The conversation history as a list of messages
    pub messages: Vec<ChatMessage>,

    /// List of sources to search for relevant context
    #[schema(default = default_retriever_sources)]
    #[serde(default = "default_retriever_sources")]
    pub sources: Vec<String>,

    /// Optional schema for structured output format
    #[schema(value_type = Option<Schema::Object>)]
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
}

fn default_think_stream() -> bool {
    true
}

/// Request schema for asking a question
#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct AskQueryRequest {
    /// The conversation history as a list of messages
    pub messages: Vec<ChatMessage>,

    /// List of sources to search for relevant context
    #[schema(default = default_retriever_sources)]
    #[serde(default = "default_retriever_sources")]
    pub sources: Vec<String>,

    /// Optional schema for structured output format
    #[schema(value_type = Option<Schema::Object>)]
    pub format: Option<chat::Schema>,
}

//------------------------------------------------------------------------------
// Embeddings Module Schemas
//------------------------------------------------------------------------------

/// Available embedding models
#[derive(ToSchema, Deserialize)]
pub enum ModelEmbed {
    #[serde(rename = "nomic-embed-text")]
    NomicEmbedTextV15,
    #[serde(rename = "nomic-embed-vision")]
    NomicEmbedVisionV15,
}

/// Request schema for generating embeddings
#[derive(ToSchema, Deserialize)]
pub struct EmbeddingsQueryRequest {
    /// The embedding model to use
    pub model: ModelEmbed,

    /// The input data to generate embeddings for (text or base64-encoded image)
    pub input: serde_json::Value,
}

//------------------------------------------------------------------------------
// Indexer Module Schemas
//------------------------------------------------------------------------------

/// Request schema for indexing content
#[derive(ToSchema, Clone, Serialize, Deserialize)]
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
#[derive(ToSchema, Clone, Serialize, Deserialize)]
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
#[derive(ToSchema, Clone, Serialize, Deserialize)]
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
#[derive(ToSchema, Clone, Serialize, Deserialize)]
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

impl Into<Index> for IndexRequest {
    fn into(self) -> Index {
        match self {
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
                let content = ImagesContent::builder().images(images.content.images).build();

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

fn query_schema() -> utoipa::openapi::schema::Object {
    use utoipa::openapi::schema::{ObjectBuilder, Type};

    ObjectBuilder::new()
        .schema_type(Type::Object)
        .property(
            "type",
            ObjectBuilder::new()
                .schema_type(Type::String)
                .enum_values(Some(["Text"]))
                .description(Some("Type of query"))
                .build(),
        )
        .required("type")
        .property("query", ObjectBuilder::new().schema_type(Type::String).description(Some("The query text")).build())
        .required("query")
        .build()
}

/// Output format for retrieval results
#[derive(ToSchema, Debug, Clone, Serialize, Deserialize)]
pub enum RetrieveOutputFormat {
    #[serde(rename = "markdown")]
    Markdown,
    #[serde(rename = "json")]
    Json,
}

/// Request schema for retrieving context
#[derive(ToSchema, Debug, Clone, Serialize, Deserialize)]
#[schema(example = json!({
    "type": "Text",
    "query": "What is Bioma?",
    "threshold": 0.0,
    "limit": 10,
    "sources": ["path/to/source1", "path/to/source2"],
    "format": "markdown"
}))]
pub struct RetrieveContextRequest {
    #[serde(flatten)]
    #[schema(schema_with = query_schema)]
    pub retrieve_context: RetrieveContext,

    /// The format of the output (markdown or json)
    #[schema(default = default_retriever_format)]
    #[serde(default = "default_retriever_format")]
    pub format: RetrieveOutputFormat,
}

fn default_retriever_format() -> RetrieveOutputFormat {
    RetrieveOutputFormat::Markdown
}

//------------------------------------------------------------------------------
// File Operations Schemas
//------------------------------------------------------------------------------

/// Metadata for file upload
#[derive(Debug, Deserialize, ToSchema)]
pub struct UploadMetadata {
    /// Target path where the file should be stored
    #[schema(value_type = String)]
    pub path: std::path::PathBuf,
}

/// Request schema for file upload
#[derive(ToSchema, Debug, MultipartForm)]
pub struct UploadRequest {
    /// The file to upload (max size: 100MB)
    #[multipart(limit = "100MB")]
    #[schema(value_type = String, format = Binary)]
    pub file: TempFile,

    /// Metadata about the upload
    #[multipart(rename = "metadata")]
    #[schema(value_type = UploadMetadata)]
    pub metadata: MpJson<UploadMetadata>,
}
