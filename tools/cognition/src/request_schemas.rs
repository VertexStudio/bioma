use actix_multipart::form::{json::Json as MpJson, tempfile::TempFile, MultipartForm};
use bioma_llm::{
    chat,
    prelude::{ChatMessage, DeleteSource, IndexGlobs, RetrieveContext, RetrieveQuery},
    rerank::{RankTexts, TruncationDirection},
};
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
#[schema(example = json!({
    "globs": ["./path/to/files/**/*.rs"], 
    "chunk_capacity": {"start": 500, "end": 2000},
    "chunk_overlap": 200
}))]
pub struct IndexGlobsRequestSchema {
    pub globs: Vec<String>,
    #[schema(value_type = ChunkCapacityRequestSchema)]
    pub chunk_capacity: Option<ChunkCapacityRequestSchema>,
    pub chunk_overlap: Option<usize>,
    pub chunk_batch_size: Option<usize>,
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
#[schema(example = json!({
    "type": "Text",
    "query": "What is Bioma?",
    "threshold": 0.0,
    "limit": 10,
    "source": ".*"
}))]
pub struct RetrieveContextRequest {
    #[schema(value_type = RetrieveQueryRequestSchema)]
    #[serde(flatten)]
    pub query: RetrieveQueryRequestSchema,
    pub limit: Option<usize>,
    pub threshold: Option<f32>,
    pub source: Option<String>,
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
            .source(self.source.unwrap_or_default())
            .build()
    }
}

// /ask Endpoint Schemas

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
#[schema(example = json!({
    "model": "llama3.2",
    "messages": [
        {
            "role": "user",
            "content": "Tell me about Puerto Rico."
        }
    ],
    "format": {
        "title": "PuertoRicoInfo",
        "type": "object",
        "required": [
            "name",
            "capital",
            "languages"
        ],
        "properties": {
            "name": {
                "description": "Name of the territory",
                "type": "string"
            },
            "capital": {
                "description": "Capital city",
                "type": "string"
            },
            "languages": {
                "description": "Official languages spoken",
                "type": "array",
                "items": {
                    "type": "string"
                }
            }
        }
    }
}))]
pub struct AskQueryRequestSchema {
    #[schema(value_type = Vec<ChatMessageRequestSchema>)]
    pub messages: Vec<ChatMessage>,
    pub source: Option<String>,
    #[schema(value_type = Schema::Object)]
    pub format: Option<chat::Schema>,
}

// /chat Endpoint Schemas

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
#[schema(example = json!({
    "model": "llama3.2",
    "messages": [
        {
            "role": "user",
            "content": "Why is the sky blue?"
        }
    ],
    "use_tools": false
}))]

pub struct ChatQueryRequestSchema {
    #[schema(value_type = Vec<ChatMessageRequestSchema>)]
    pub messages: Vec<ChatMessage>,
    pub source: Option<String>,
    #[schema(value_type = Schema::Object)]
    pub format: Option<chat::Schema>,
    pub use_tools: bool,
}

// /delete_resource Endpoint Schemas

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
#[schema(example = json!({"source": "path/to/source1"}))]
pub struct DeleteSourceRequestSchema {
    pub source: String,
}

impl Into<DeleteSource> for DeleteSourceRequestSchema {
    fn into(self) -> DeleteSource {
        DeleteSource { source: self.source }
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
#[schema(example = json!({
    "input": "This text will generate embeddings",
    "model": "nomic-embed-text"
  }))]
#[schema(example = json!({
    "model": "nomic-embed-vision",
    "input": "iVBORw0KGgoAAAANSUhEUgAAAAoAAAAKCAIAAAACUFjqAAABRklEQVR4nAA2Acn+A2ql2+Vv1LF7X3Mw2i9cMEBUs0/l0C6/irfF6wPqowTw0ORE00EZ/He1x+LwZ3nDwaZVNIgn6FI8KQabKikArD0j4g6LU2Mz9DpsAgnYGy6195whWQQ4XIk1a74tA98BtQfyE3oQkaA/uufBkIegK+TH6LMh/O44hIio5wAw4umxtkxZNCIf35A4YNshDwNeeHFnHP0YUSelrm8DMioFvjc7QOcZmEBw/pv+SXEH2G+O0ZdiHDTb6wnhAcRk1rkuJLwy/d7DDKTgqOflV5zk7IBgmz0f8J4o5gA4yb3rYzzUyLRXS0bY40xnoY/rtniWFdlrtSHkR/0A1ClG/qVWNyD1CXVkxE4IW5Tj+8qk1sD42XW6TQpPAO7NhmcDxDz092Q2AR8XYKPa1LPkGberOYArt0gkbQEAAP//4hWZNZ4Pc4kAAAAASUVORK5CYII="
}))]
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
#[schema(example = json!({
    "query": "What is Deep Learning?",
    "texts": [
        "Deep Learning is learning under water",
        "Deep learning is a branch of machine learning"
    ],
    "raw_scores": false
}))]
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
