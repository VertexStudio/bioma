use std::{path::PathBuf, vec};

use actix_multipart::form::{json::Json as MpJson, tempfile::TempFile, MultipartForm};
use bioma_llm::{
    chat,
    prelude::{ChatMessage, DeleteSource, Image, IndexGlobs, RetrieveContext, RetrieveQuery},
    rerank::{RankTexts, TruncationDirection},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::{AskQuery, ChatQuery, Upload, UploadMetadata};

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

impl Into<ChatMessage> for ChatMessageRequestSchema {
    fn into(self) -> ChatMessage {
        let images: Vec<Image> = match self.images {
            Some(images) => images.into_iter().map(|image| Image::from_base64(image)).collect(),
            None => vec![],
        };

        let role = match self.role {
            MessageRoleRequestSchema::Assistant => bioma_llm::prelude::MessageRole::Assistant,
            MessageRoleRequestSchema::System => bioma_llm::prelude::MessageRole::System,
            MessageRoleRequestSchema::User => bioma_llm::prelude::MessageRole::User,
            MessageRoleRequestSchema::Tool => bioma_llm::prelude::MessageRole::Tool,
        };

        ChatMessage::new(role, self.content).with_images(images)
    }
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

// ask

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct AskQueryRequestSchema {
    pub messages: Vec<ChatMessageRequestSchema>,
    pub source: Option<String>,
    pub format: Option<Value>,
}

impl TryInto<AskQuery> for AskQueryRequestSchema {
    type Error = String;

    fn try_into(self) -> Result<AskQuery, Self::Error> {
        let messages: Vec<ChatMessage> = self.messages.into_iter().map(|message| message.into()).collect();

        let format: Option<chat::Schema> = match self.format {
            Some(format) => match serde_json::from_value(format) {
                Ok(format) => Some(format),
                Err(_) => return Err("\"format\" field structure is not valid".to_string()),
            },
            None => None,
        };

        Ok(AskQuery { format, source: self.source, messages })
    }
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ChatQueryRequestSchema {
    pub messages: Vec<ChatMessageRequestSchema>,
    pub source: Option<String>,
    pub format: Option<Value>,
}

impl TryInto<ChatQuery> for ChatQueryRequestSchema {
    type Error = String;

    fn try_into(self) -> Result<ChatQuery, Self::Error> {
        let messages: Vec<ChatMessage> = self.messages.into_iter().map(|message| message.into()).collect();

        let format: Option<chat::Schema> = match self.format {
            Some(format) => match serde_json::from_value(format) {
                Ok(format) => Some(format),
                Err(_) => return Err("\"format\" field structure is not valid".to_string()),
            },
            None => None,
        };

        Ok(ChatQuery { format, source: self.source, messages })
    }
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct DeleteSourceRequestSchema {
    pub source: String,
}

impl Into<DeleteSource> for DeleteSourceRequestSchema {
    fn into(self) -> DeleteSource {
        DeleteSource { source: self.source }
    }
}

#[derive(ToSchema, Deserialize)]
pub struct EmbeddingsQueryRequestSchema {
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

#[derive(ToSchema, Debug, Deserialize)]
pub struct UploadMetadataRequestSchema {
    pub path: String,
}

impl Into<UploadMetadata> for UploadMetadataRequestSchema {
    fn into(self) -> UploadMetadata {
        UploadMetadata { path: PathBuf::from(self.path) }
    }
}

#[derive(ToSchema, Debug, MultipartForm)]
pub struct UploadRequestSchema {
    #[multipart(limit = "100MB")]
    #[schema(value_type = String, format = Binary, content_media_type = "application/octet-stream")]
    pub file: TempFile,
    #[schema(value_type = UploadMetadataRequestSchema)]
    #[multipart(rename = "metadata")]
    pub metadata: MpJson<UploadMetadataRequestSchema>,
}

impl Into<Upload> for UploadRequestSchema {
    fn into(self) -> Upload {
        let upload_metadata: UploadMetadata = self.metadata.into_inner().into();
        let metadata: MpJson<UploadMetadata> = MpJson(upload_metadata);

        Upload { file: self.file, metadata }
    }
}
