use actix_multipart::form::{json::Json as MpJson, tempfile::TempFile, MultipartForm};
use bioma_llm::{
    chat,
    prelude::{ChatMessage, ChatMessageResponse, DeleteSource, RetrieveContext, RetrieveQuery},
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
