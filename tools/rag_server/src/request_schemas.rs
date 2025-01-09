use std::vec;

use bioma_llm::{
    chat,
    prelude::{ChatMessage, Image, IndexGlobs, RetrieveContext, RetrieveQuery},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AskQuery;

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
pub struct IndexGlobsRequest {
    pub globs: Vec<String>,
    #[schema(value_type = ChunkCapacity)]
    pub chunk_capacity: Option<ChunkCapacity>,
    pub chunk_overlap: Option<usize>,
    pub chunk_batch_size: Option<usize>,
}

#[derive(ToSchema, Clone, Serialize, Deserialize)]
pub struct ChunkCapacity {
    pub start: usize,
    pub end: usize,
}

impl Into<IndexGlobs> for IndexGlobsRequest {
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

pub enum RetrieveQueryRequest {
    #[serde(rename = "query")]
    Text(String),
    Segundo(String),
}

#[derive(ToSchema, Debug, Clone, Serialize, Deserialize)]
pub struct RetrieveContextRequest {
    #[schema(value_type = RetrieveQueryRequest)]
    #[serde(flatten)]
    pub query: RetrieveQueryRequest,
    pub limit: Option<usize>,
    pub threshold: Option<f32>,
    pub source: Option<String>,
}

impl Into<RetrieveContext> for RetrieveContextRequest {
    fn into(self) -> RetrieveContext {
        let query = match self.query {
            RetrieveQueryRequest::Text(query) => RetrieveQuery::Text(query),
            RetrieveQueryRequest::Segundo(query) => RetrieveQuery::Text(query),
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
pub struct AskQueryRequest {
    pub messages: Vec<ChatMessageRequestSchema>,
    pub source: Option<String>,
    // pub format: Option<chat::Schema>,
}

impl Into<AskQuery> for AskQueryRequest {
    fn into(self) -> AskQuery {
        let messages: Vec<ChatMessage> = self.messages.into_iter().map(|message| message.into()).collect();

        // let format

        AskQuery { format: None, source: self.source, messages }
    }
}
