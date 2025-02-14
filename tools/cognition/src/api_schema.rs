use actix_multipart::form::{json::Json as MpJson, tempfile::TempFile, MultipartForm};
use bioma_llm::{
    chat,
    prelude::{ChatMessage, DeleteSource, IndexGlobs, RetrieveContext, RetrieveQuery},
    rerank::{RankTexts, TruncationDirection},
};
use ollama_rs::generation::tools::ToolInfo;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// /index Endpoint Schemas

const DEFAULT_CHUNK_CAPACITY: std::ops::Range<usize> = 500..2000;
const DEFAULT_CHUNK_OVERLAP: usize = 200;
const DEFAULT_CHUNK_BATCH_SIZE: usize = 50;

#[derive(ToSchema, Clone, Serialize, Deserialize, Debug)]
pub enum MessageRoleRequestSchema {
    #[serde(rename = "user")]
    User,
    #[serde(rename = "assistant")]
    Assistant,
    #[serde(rename = "system")]
    System,
    #[serde(rename = "tool")]
    Tool,
}

#[derive(ToSchema, Clone, Serialize, Deserialize, Debug)]
pub struct ChatMessageRequestSchema {
    #[schema(value_type = MessageRoleRequestSchema)]
    pub role: MessageRoleRequestSchema,
    pub content: String,
    pub images: Option<Vec<String>>,
}

#[derive(ToSchema, Clone, Serialize, Deserialize)]
pub struct IndexGlobsRequestSchema {
    #[schema(default = default_source)]
    #[serde(default = "default_source")]
    pub source: String,
    pub globs: Vec<String>,
    #[schema(value_type = ChunkCapacityRequestSchema)]
    pub chunk_capacity: Option<ChunkCapacityRequestSchema>,
    pub chunk_overlap: Option<usize>,
    pub chunk_batch_size: Option<usize>,
}

fn default_source() -> String {
    "/global".to_string()
}

#[derive(ToSchema, Clone, Serialize, Deserialize)]
pub struct ChunkCapacityRequestSchema {
    pub start: usize,
    pub end: usize,
}

impl Into<IndexGlobs> for IndexGlobsRequestSchema {
    fn into(self) -> IndexGlobs {
        let chunk_capacity = match self.chunk_capacity {
            Some(capacity) => capacity.start..capacity.end,
            None => DEFAULT_CHUNK_CAPACITY,
        };

        IndexGlobs::builder()
            .source(self.source)
            .globs(self.globs)
            .chunk_capacity(chunk_capacity)
            .chunk_overlap(self.chunk_overlap.unwrap_or(DEFAULT_CHUNK_OVERLAP))
            .chunk_batch_size(self.chunk_batch_size.unwrap_or(DEFAULT_CHUNK_BATCH_SIZE))
            .build()
    }
}

// /retrive Endpoint Schemas

const DEFAULT_RETRIEVER_LIMIT: usize = 10;
const DEFAULT_RETRIEVER_THRESHOLD: f32 = 0.0;

#[derive(ToSchema, Debug, Clone, Serialize, Deserialize)]

pub enum RetrieveQueryRequestSchema {
    #[serde(rename = "query")]
    Text(String),
    Segundo(String),
}

#[derive(ToSchema, Debug, Clone, Serialize, Deserialize)]
pub enum RetrieveOutputFormat {
    #[serde(rename = "markdown")]
    Markdown,
    #[serde(rename = "json")]
    Json,
}

impl Default for RetrieveOutputFormat {
    fn default() -> Self {
        RetrieveOutputFormat::Markdown
    }
}

#[derive(ToSchema, Debug, Clone, Serialize, Deserialize)]
pub struct RetrieveContextRequest {
    #[schema(value_type = RetrieveQueryRequestSchema)]
    #[serde(flatten)]
    pub query: RetrieveQueryRequestSchema,
    pub limit: Option<usize>,
    pub threshold: Option<f32>,
    #[schema(default = default_sources)]
    #[serde(default = "default_sources")]
    pub sources: Vec<String>,
    #[serde(default)]
    pub format: RetrieveOutputFormat,
}

fn default_sources() -> Vec<String> {
    vec!["/global".to_string()]
}

impl Into<RetrieveContext> for RetrieveContextRequest {
    fn into(self) -> RetrieveContext {
        let query = match self.query {
            RetrieveQueryRequestSchema::Text(query) => RetrieveQuery::Text(query),
            RetrieveQueryRequestSchema::Segundo(query) => RetrieveQuery::Text(query),
        };

        RetrieveContext::builder()
            .query(query)
            .limit(self.limit.unwrap_or(DEFAULT_RETRIEVER_LIMIT))
            .threshold(self.threshold.unwrap_or(DEFAULT_RETRIEVER_THRESHOLD))
            .sources(self.sources)
            .build()
    }
}

pub fn non_empty_messages<'de, D>(deserializer: D) -> Result<Vec<ChatMessage>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let messages = Vec::<ChatMessage>::deserialize(deserializer)?;
    if messages.is_empty() {
        return Err(serde::de::Error::custom("messages cannot be empty"));
    }
    Ok(messages)
}

// /ask Endpoint Schemas

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct AskQueryRequestSchema {
    #[schema(value_type = Vec<ChatMessageRequestSchema>)]
    #[schema(min_items = 1, example = json!([{"role": "user", "content": "Hello"}]))]
    #[serde(deserialize_with = "non_empty_messages")]
    pub messages: Vec<ChatMessage>,
    #[schema(default = default_sources)]
    #[serde(default = "default_sources")]
    pub sources: Vec<String>,
    #[schema(value_type = Schema::Object)]
    pub format: Option<chat::Schema>,
}

// /chat Endpoint Schemas

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ChatQueryRequestSchema {
    #[schema(value_type = Vec<ChatMessageRequestSchema>)]
    #[schema(min_items = 1, example = json!([{"role": "user", "content": "Hello"}]))]
    #[serde(deserialize_with = "non_empty_messages")]
    pub messages: Vec<ChatMessage>,
    #[schema(default = default_sources)]
    #[serde(default = "default_sources")]
    pub sources: Vec<String>,
    #[schema(value_type = Schema::Object)]
    pub format: Option<chat::Schema>,
    #[serde(default)]
    pub tools: Vec<ToolInfoSchema>,
    #[serde(default)]
    pub tools_actors: Vec<String>,
    #[serde(default = "default_chat_stream")]
    pub stream: bool,
}

fn default_chat_stream() -> bool {
    true
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ThinkQueryRequestSchema {
    #[schema(value_type = Vec<ChatMessageRequestSchema>)]
    #[schema(min_items = 1, example = json!([{"role": "user", "content": "Hello"}]))]
    #[serde(deserialize_with = "non_empty_messages")]
    pub messages: Vec<ChatMessage>,
    #[schema(default = default_sources)]
    #[serde(default = "default_sources")]
    pub sources: Vec<String>,
    #[schema(value_type = Schema::Object)]
    pub format: Option<chat::Schema>,
    #[serde(default)]
    pub tools: Vec<ToolInfoSchema>,
    #[serde(default)]
    // TODO: Vec<ActorId> or Vec<String>? The first one requires custom deserialization.
    pub tools_actors: Vec<String>,
    #[serde(default = "default_think_stream")]
    pub stream: bool,
}

fn default_think_stream() -> bool {
    true
}

// /delete_resource Endpoint Schemas

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct DeleteSourceRequestSchema {
    pub sources: Vec<String>,
}

impl Into<DeleteSource> for DeleteSourceRequestSchema {
    fn into(self) -> DeleteSource {
        DeleteSource { sources: self.sources }
    }
}

// /embed Endpoint Schemas

#[derive(ToSchema, Deserialize)]
pub enum ModelEmbedRequestSchema {
    #[serde(rename = "nomic-embed-text")]
    NomicEmbedTextV15,
    #[serde(rename = "nomic-embed-vision")]
    NomicEmbedVisionV15,
}

#[derive(ToSchema, Deserialize)]
pub struct EmbeddingsQueryRequestSchema {
    #[schema(value_type = ModelEmbedRequestSchema)]
    pub model: String,
    pub input: serde_json::Value,
}

// /rerank Endpoint Schemas

#[derive(ToSchema, Debug, Clone, Serialize, Deserialize)]
pub enum TruncationDirectionRequestSchema {
    #[serde(rename = "left")]
    Left,
    #[serde(rename = "right")]
    Right,
}
#[derive(ToSchema, Debug, Clone, Serialize, Deserialize)]
pub struct RankTextsRequestSchema {
    pub query: String,
    pub raw_scores: Option<bool>,
    pub return_text: Option<bool>,
    pub texts: Vec<String>,
    pub truncate: Option<bool>,
    #[schema(value_type = TruncationDirectionRequestSchema)]
    pub truncation_direction: Option<TruncationDirectionRequestSchema>,
}

impl Into<RankTexts> for RankTextsRequestSchema {
    fn into(self) -> RankTexts {
        let truncation_direction = match self.truncation_direction {
            Some(TruncationDirectionRequestSchema::Left) => TruncationDirection::Left,
            Some(TruncationDirectionRequestSchema::Right) => TruncationDirection::Right,
            None => TruncationDirection::Right,
        };

        RankTexts {
            query: self.query,
            raw_scores: self.raw_scores.unwrap_or(false),
            return_text: self.return_text.unwrap_or(false),
            texts: self.texts,
            truncate: self.truncate.unwrap_or(false),
            truncation_direction,
        }
    }
}

// /upload Endpoint Schemas

#[derive(Debug, Deserialize, ToSchema)]
pub struct UploadMetadata {
    #[schema(value_type = String, format = Binary, content_media_type = "application/octet-stream")]
    pub path: std::path::PathBuf,
}

#[derive(ToSchema, Debug, MultipartForm)]
pub struct UploadRequestSchema {
    #[multipart(limit = "100MB")]
    #[schema(value_type = String, format = Binary, content_media_type = "application/octet-stream")]
    pub file: TempFile,
    #[multipart(rename = "metadata")]
    #[schema(value_type = UploadMetadata)]
    pub metadata: MpJson<UploadMetadata>,
}

// Add these new type definitions that implement ToSchema
#[derive(ToSchema, Clone, Serialize, Deserialize, Debug)]
pub enum ToolTypeSchema {
    #[serde(rename = "function")]
    Function,
}

#[derive(ToSchema, Clone, Serialize, Deserialize, Debug)]
pub struct ToolFunctionInfoSchema {
    pub name: String,
    pub description: String,
    #[schema(value_type = Schema::Object)]
    pub parameters: schemars::schema::RootSchema,
}

#[derive(ToSchema, Clone, Serialize, Deserialize, Debug)]
pub struct ToolInfoSchema {
    #[serde(rename = "type")]
    pub tool_type: ToolTypeSchema,
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
