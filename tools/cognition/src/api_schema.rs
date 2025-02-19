use actix_multipart::form::{json::Json as MpJson, tempfile::TempFile, MultipartForm};
use bioma_llm::{
    chat,
    indexer::{
        default_chunk_batch_size, default_chunk_overlap, GlobsContent, ImagesContent,
        TextChunkConfig as BiomaTextChunkConfig, TextsContent, DEFAULT_CHUNK_CAPACITY,
    },
    prelude::{ChatMessage, ChatMessageResponse, DeleteSource, Index, IndexContent, RetrieveContext, RetrieveQuery},
    rerank::{default_raw_scores, default_return_text, default_truncate, RankTexts, TruncationDirection},
    retriever::{default_retriever_limit, default_retriever_sources, default_retriever_threshold},
};
use ollama_rs::generation::{
    chat::ChatMessageFinalResponseData,
    tools::{ToolCall, ToolInfo},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

//------------------------------------------------------------------------------
// Common Types
//------------------------------------------------------------------------------

#[derive(ToSchema, Debug, Serialize)]
pub struct ChatMessageResponseSchema {
    /// The name of the model used for the completion.
    pub model: String,
    /// The creation time of the completion, in such format: `2023-08-04T08:52:19.385406455-07:00`.
    pub created_at: String,
    /// The generated chat message.
    #[schema(value_type = ChatMessageSchema)]
    pub message: ChatMessage,
    /// Whether the response is complete
    pub done: bool,
    #[serde(flatten)]
    /// The final data of the completion. This is only present if the completion is done.
    #[schema(value_type = Option<ChatMessageFinalResponseDataSchema>)]
    pub final_data: Option<ChatMessageFinalResponseData>,
}

/// Role of a message in a chat conversation
#[derive(ToSchema, Clone, Serialize, Deserialize, Debug)]
#[schema(title = "MessageRole")]
pub enum MessageRoleSchema {
    #[serde(rename = "user")]
    User,
    #[serde(rename = "assistant")]
    Assistant,
    #[serde(rename = "system")]
    System,
    #[serde(rename = "tool")]
    Tool,
}

/// A single message in a chat conversation
#[derive(ToSchema, Clone, Serialize, Deserialize, Debug)]
#[schema(title = "ChatMessage")]
pub struct ChatMessageSchema {
    /// The role of the message sender (user, assistant, system, or tool)
    #[schema(value_type = MessageRoleSchema)]
    pub role: MessageRoleSchema,

    /// The content of the message
    pub content: String,

    /// Optional list of tool calls attached to the message
    #[schema(value_type = Option<Vec<Object>>)]
    pub tool_calls: Option<Vec<ToolCall>>,

    /// Optional list of base64-encoded images attached to the message
    pub images: Option<Vec<String>>,
}

//------------------------------------------------------------------------------
// Chat Module Schemas
//------------------------------------------------------------------------------

/// Request schema for chat completion
#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ChatQuery {
    /// The conversation history as a list of messages
    #[schema(value_type = Vec<ChatMessageSchema>)]
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
    pub tools: Vec<ToolInfoSchema>,

    /// List of tool actor identifiers
    #[serde(default)]
    pub tools_actors: Vec<String>,

    /// Whether to stream the response
    #[serde(default = "default_chat_stream")]
    pub stream: bool,
}

/// Response schema for chat completion
#[derive(ToSchema, Debug, Serialize)]
pub struct ChatResponseSchema {
    #[schema(value_type = ChatMessageResponseSchema)]
    #[serde(flatten)]
    pub response: ChatMessageResponse,

    /// The conversation context used to generate the response
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[schema(value_type = Vec<ChatMessageSchema>)]
    pub context: Vec<ChatMessage>,
}

/// Final statistics about a chat completion
#[derive(ToSchema, Debug, Serialize)]
pub struct ChatMessageFinalResponseDataSchema {
    /// Time spent generating the response in nanoseconds
    pub total_duration: u64,

    /// Number of tokens in the prompt
    pub prompt_eval_count: u16,

    /// Time spent evaluating the prompt in nanoseconds
    pub prompt_eval_duration: u64,

    /// Number of tokens in the response
    pub eval_count: u16,

    /// Time spent generating the response in nanoseconds
    pub eval_duration: u64,
}

fn default_chat_stream() -> bool {
    true
}

/// Request schema for think operation
#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ThinkQueryRequestSchema {
    /// The conversation history as a list of messages
    #[schema(value_type = Vec<ChatMessageSchema>)]
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
    pub tools: Vec<ToolInfoSchema>,

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
pub struct AskQueryRequestSchema {
    /// The conversation history as a list of messages
    #[schema(value_type = Vec<ChatMessageSchema>)]
    pub messages: Vec<ChatMessage>,

    /// List of sources to search for relevant context
    #[schema(default = default_retriever_sources)]
    #[serde(default = "default_retriever_sources")]
    pub sources: Vec<String>,

    /// Optional schema for structured output format
    #[schema(value_type = Option<Schema::Object>)]
    pub format: Option<chat::Schema>,
}

/// Response schema for ask operation
#[derive(ToSchema, Debug, Serialize)]
pub struct AskResponseSchema {
    #[schema(value_type = ChatMessageResponseSchema)]
    #[serde(flatten)]
    pub response: ChatMessageResponse,

    /// The conversation context used to generate the response
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[schema(value_type = Vec<ChatMessageSchema>)]
    pub context: Vec<ChatMessage>,
}

//------------------------------------------------------------------------------
// Embeddings Module Schemas
//------------------------------------------------------------------------------

/// Available embedding models
#[derive(ToSchema, Deserialize)]
pub enum ModelEmbedRequestSchema {
    #[serde(rename = "nomic-embed-text")]
    NomicEmbedTextV15,
    #[serde(rename = "nomic-embed-vision")]
    NomicEmbedVisionV15,
}

/// Request schema for generating embeddings
#[derive(ToSchema, Deserialize)]
pub struct EmbeddingsQueryRequestSchema {
    /// The embedding model to use
    #[schema(value_type = ModelEmbedRequestSchema)]
    pub model: String,

    /// The input data to generate embeddings for (text or base64-encoded image)
    pub input: serde_json::Value,
}

//------------------------------------------------------------------------------
// Indexer Module Schemas
//------------------------------------------------------------------------------

/// Request schema for indexing content
#[derive(ToSchema, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IndexRequestSchema {
    /// Index content using glob patterns
    Globs(IndexGlobsRequestSchema),
    /// Index text content directly
    Texts(IndexTextsRequestSchema),
    /// Index image content
    Images(IndexImagesRequestSchema),
}

/// Request schema for indexing files using glob patterns
#[derive(ToSchema, Clone, Serialize, Deserialize)]
pub struct IndexGlobsRequestSchema {
    /// The source identifier for the indexed content
    #[schema(default = default_source)]
    #[serde(default = "default_source")]
    pub source: String,

    /// List of glob patterns to match files for indexing
    #[schema(example = json!(["./path/to/files/**/*.rs"]))]
    pub globs: Vec<String>,

    /// Configuration for text chunk size limits
    #[schema(example = json!({"start": 500, "end": 2000}))]
    #[serde(default = "default_chunk_capacity")]
    pub chunk_capacity: ChunkCapacityRequestSchema,

    /// Number of tokens to overlap between chunks
    #[schema(default = default_chunk_overlap, minimum = 0)]
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap: usize,

    /// Number of chunks to process in each batch
    #[schema(default = default_chunk_batch_size, minimum = 0)]
    #[serde(default = "default_chunk_batch_size")]
    pub chunk_batch_size: usize,

    /// Whether to summarize each file
    #[schema(default = false)]
    #[serde(default)]
    pub summarize: bool,
}

/// Request schema for indexing text content directly
#[derive(ToSchema, Clone, Serialize, Deserialize)]
pub struct IndexTextsRequestSchema {
    /// The source identifier for the indexed content
    #[schema(default = default_source)]
    #[serde(default = "default_source")]
    pub source: String,

    /// List of texts to index
    pub texts: Vec<String>,

    /// MIME type for the texts
    #[schema(default = default_text_mime_type)]
    #[serde(default = "default_text_mime_type")]
    pub mime_type: String,

    /// Configuration for text chunk size limits
    #[schema(example = json!({"start": 500, "end": 2000}))]
    #[serde(default = "default_chunk_capacity")]
    pub chunk_capacity: ChunkCapacityRequestSchema,

    /// Number of tokens to overlap between chunks
    #[schema(default = default_chunk_overlap, minimum = 0)]
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap: usize,

    /// Number of chunks to process in each batch
    #[schema(default = default_chunk_batch_size, minimum = 0)]
    #[serde(default = "default_chunk_batch_size")]
    pub chunk_batch_size: usize,

    /// Whether to summarize each text
    #[schema(default = false)]
    #[serde(default)]
    pub summarize: bool,
}

/// Request schema for indexing image content
#[derive(ToSchema, Clone, Serialize, Deserialize)]
pub struct IndexImagesRequestSchema {
    /// The source identifier for the indexed content
    #[schema(default = default_source)]
    #[serde(default = "default_source")]
    pub source: String,

    /// List of base64 encoded images
    pub images: Vec<String>,

    /// Optional MIME type for the images
    #[serde(default)]
    pub mime_type: Option<String>,

    /// Whether to summarize each image
    #[schema(default = false)]
    #[serde(default)]
    pub summarize: bool,
}

fn default_text_mime_type() -> String {
    "text/plain".to_string()
}

fn default_source() -> String {
    "/global".to_string()
}

fn default_chunk_capacity() -> ChunkCapacityRequestSchema {
    ChunkCapacityRequestSchema { start: DEFAULT_CHUNK_CAPACITY.start, end: DEFAULT_CHUNK_CAPACITY.end }
}

/// Configuration for text chunk capacity
#[derive(ToSchema, Clone, Serialize, Deserialize)]
pub struct ChunkCapacityRequestSchema {
    /// Minimum number of tokens in a chunk
    #[schema(minimum = 0)]
    pub start: usize,

    /// Maximum number of tokens in a chunk
    #[schema(minimum = 0)]
    pub end: usize,
}

impl Into<Index> for IndexRequestSchema {
    fn into(self) -> Index {
        match self {
            IndexRequestSchema::Globs(globs) => {
                let chunk_capacity =
                    std::ops::Range { start: globs.chunk_capacity.start, end: globs.chunk_capacity.end };

                Index::builder()
                    .source(globs.source)
                    .content(IndexContent::Globs(
                        GlobsContent::builder()
                            .globs(globs.globs)
                            .config(
                                BiomaTextChunkConfig::builder()
                                    .chunk_capacity(chunk_capacity)
                                    .chunk_overlap(globs.chunk_overlap)
                                    .chunk_batch_size(globs.chunk_batch_size)
                                    .build(),
                            )
                            .build(),
                    ))
                    .summarize(globs.summarize)
                    .build()
            }
            IndexRequestSchema::Texts(texts) => {
                let chunk_capacity =
                    std::ops::Range { start: texts.chunk_capacity.start, end: texts.chunk_capacity.end };

                Index::builder()
                    .source(texts.source)
                    .content(IndexContent::Texts(
                        TextsContent::builder()
                            .texts(texts.texts)
                            .mime_type(texts.mime_type)
                            .config(
                                BiomaTextChunkConfig::builder()
                                    .chunk_capacity(chunk_capacity)
                                    .chunk_overlap(texts.chunk_overlap)
                                    .chunk_batch_size(texts.chunk_batch_size)
                                    .build(),
                            )
                            .build(),
                    ))
                    .summarize(texts.summarize)
                    .build()
            }
            IndexRequestSchema::Images(images) => {
                let content = if let Some(mime) = images.mime_type {
                    ImagesContent::builder().images(images.images).mime_type(mime).build()
                } else {
                    ImagesContent::builder().images(images.images).build()
                };

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

/// Query types for retrieval
#[derive(ToSchema, Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "query")]
pub enum RetrieveQueryRequestSchema {
    /// Text-based query
    #[serde(rename = "Text")]
    Text(String),
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
    /// The query to search for
    #[schema(schema_with = query_schema)]
    #[serde(flatten)]
    pub query: RetrieveQueryRequestSchema,

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

fn default_retriever_format() -> RetrieveOutputFormat {
    RetrieveOutputFormat::Markdown
}

impl Into<RetrieveContext> for RetrieveContextRequest {
    fn into(self) -> RetrieveContext {
        let query = match self.query {
            RetrieveQueryRequestSchema::Text(query) => RetrieveQuery::Text(query),
        };

        RetrieveContext::builder()
            .query(query)
            .limit(self.limit)
            .threshold(self.threshold)
            .sources(self.sources)
            .build()
    }
}

//------------------------------------------------------------------------------
// Rerank Module Schemas
//------------------------------------------------------------------------------

/// Direction for text truncation
#[derive(ToSchema, Debug, Clone, Serialize, Deserialize)]
pub enum TruncationDirectionRequestSchema {
    #[serde(rename = "left")]
    Left,
    #[serde(rename = "right")]
    Right,
}

/// Request schema for ranking texts
#[derive(ToSchema, Debug, Clone, Serialize, Deserialize)]
pub struct RankTextsRequestSchema {
    /// The query to compare texts against
    pub query: String,

    /// Whether to return raw similarity scores
    #[schema(default = default_raw_scores)]
    #[serde(default = "default_raw_scores")]
    pub raw_scores: bool,

    /// Whether to include the text content in the response
    #[schema(default = default_return_text)]
    #[serde(default = "default_return_text")]
    pub return_text: bool,

    /// List of texts to rank
    pub texts: Vec<String>,

    /// Whether to truncate texts
    #[schema(default = default_truncate)]
    #[serde(default = "default_truncate")]
    pub truncate: bool,

    /// Direction to truncate texts from
    #[schema(default = default_truncation_direction)]
    #[serde(default = "default_truncation_direction")]
    pub truncation_direction: TruncationDirectionRequestSchema,
}

fn default_truncation_direction() -> TruncationDirectionRequestSchema {
    TruncationDirectionRequestSchema::Right
}

impl Into<RankTexts> for RankTextsRequestSchema {
    fn into(self) -> RankTexts {
        let truncation_direction = match self.truncation_direction {
            TruncationDirectionRequestSchema::Left => TruncationDirection::Left,
            TruncationDirectionRequestSchema::Right => TruncationDirection::Right,
        };

        RankTexts {
            query: self.query,
            raw_scores: self.raw_scores,
            return_text: self.return_text,
            texts: self.texts,
            truncate: self.truncate,
            truncation_direction,
        }
    }
}

//------------------------------------------------------------------------------
// File Operations Schemas
//------------------------------------------------------------------------------

/// Request schema for deleting indexed sources
#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct DeleteSourceRequestSchema {
    /// List of source identifiers to delete
    pub sources: Vec<String>,
}

impl Into<DeleteSource> for DeleteSourceRequestSchema {
    fn into(self) -> DeleteSource {
        DeleteSource { sources: self.sources }
    }
}

/// Metadata for file upload
#[derive(Debug, Deserialize, ToSchema)]
pub struct UploadMetadata {
    /// Target path where the file should be stored
    #[schema(value_type = String)]
    pub path: std::path::PathBuf,
}

/// Request schema for file upload
#[derive(ToSchema, Debug, MultipartForm)]
pub struct UploadRequestSchema {
    /// The file to upload (max size: 100MB)
    #[multipart(limit = "100MB")]
    #[schema(value_type = String, format = Binary)]
    pub file: TempFile,

    /// Metadata about the upload
    #[multipart(rename = "metadata")]
    #[schema(value_type = UploadMetadata)]
    pub metadata: MpJson<UploadMetadata>,
}

//------------------------------------------------------------------------------
// Tool Schemas
//------------------------------------------------------------------------------

/// Type of tool available
#[derive(ToSchema, Clone, Serialize, Deserialize, Debug)]
pub enum ToolTypeSchema {
    #[serde(rename = "function")]
    Function,
}

/// Information about a tool's function
#[derive(ToSchema, Clone, Serialize, Deserialize, Debug)]
pub struct ToolFunctionInfoSchema {
    /// Name of the function
    pub name: String,

    /// Description of what the function does
    pub description: String,

    /// JSON Schema describing the function parameters
    #[schema(value_type = Schema::Object)]
    pub parameters: schemars::schema::RootSchema,
}

/// Complete tool information
#[derive(ToSchema, Clone, Serialize, Deserialize, Debug)]
#[schema(title = "ToolInfo")]
pub struct ToolInfoSchema {
    /// Type of the tool
    #[serde(rename = "type")]
    pub tool_type: ToolTypeSchema,

    /// Function information for the tool
    pub function: ToolFunctionInfoSchema,
}

impl From<ToolInfoSchema> for ToolInfo {
    fn from(schema: ToolInfoSchema) -> Self {
        ToolInfo::from_schema(
            schema.function.name.into(),
            schema.function.description.into(),
            schema.function.parameters,
        )
    }
}
