use crate::embeddings::{self, Embeddings, EmbeddingsError};
use crate::indexer::{ChunkMetadata, ImageMetadata};
use crate::rerank::{RankTexts, Rerank, RerankError};
use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use tracing::{error, info};

const DEFAULT_RETRIEVER_TAG: &str = "indexer_content";
const DEFAULT_RETRIEVER_LIMIT: usize = 10;
const DEFAULT_RETRIEVER_THRESHOLD: f32 = 0.0;

#[derive(thiserror::Error, Debug)]
pub enum RetrieverError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Embeddings error: {0}")]
    Embeddings(#[from] EmbeddingsError),
    #[error("Rerank error: {0}")]
    Rerank(#[from] RerankError),
    #[error("Glob error: {0}")]
    Glob(#[from] glob::GlobError),
    #[error("Pattern error: {0}")]
    Pattern(#[from] glob::PatternError),
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error("Similarity fetch error: {0}")]
    ComputingSimilarity(String),
    #[error("Rerank error: {0}")]
    ComputingRerank(String),
    #[error("Embeddings ID not found")]
    EmbeddingsIdNotFound,
    #[error("Rerank ID not found")]
    RerankIdNotFound,
}

impl ActorError for RetrieverError {}

#[derive(bon::Builder, Debug, Clone, Serialize, Deserialize)]
pub struct RetrieveContext {
    #[serde(flatten)]
    pub query: QueryType,
    #[builder(default = DEFAULT_RETRIEVER_LIMIT)]
    #[serde(default = "default_retriever_limit")]
    pub limit: usize,
    #[builder(default = DEFAULT_RETRIEVER_THRESHOLD)]
    #[serde(default = "default_retriever_threshold")]
    pub threshold: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "content")]
pub enum QueryType {
    Text(String),
    Image(String),
}

fn default_retriever_limit() -> usize {
    DEFAULT_RETRIEVER_LIMIT
}

fn default_retriever_threshold() -> f32 {
    DEFAULT_RETRIEVER_THRESHOLD
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context {
    pub text: Option<String>,
    #[serde(flatten)]
    pub metadata: Option<ContextMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContextMetadata {
    Text(ChunkMetadata),
    Image(ImageMetadata),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievedContext {
    pub context: Vec<Context>,
}

impl RetrievedContext {
    pub fn to_markdown(&self) -> String {
        let mut context_content = String::new();

        for (index, context) in self.context.iter().enumerate() {
            // Add metadata information based on type
            if let Some(metadata) = &context.metadata {
                match metadata {
                    ContextMetadata::Text(text_metadata) => {
                        context_content.push_str(&format!("Source: {}\n", text_metadata.source));
                        context_content.push_str(&format!("Type: {}\n", text_metadata.text_type));
                        context_content.push_str(&format!("Chunk: {}\n\n", text_metadata.chunk_number));
                    }
                    ContextMetadata::Image(image_metadata) => {
                        context_content.push_str(&format!("Source: {}\n", image_metadata.source));
                        context_content.push_str(&format!("Format: {}\n", image_metadata.format));
                        context_content.push_str(&format!(
                            "Dimensions: {}x{}\n",
                            image_metadata.dimensions.width, image_metadata.dimensions.height
                        ));
                        context_content.push_str(&format!("Size: {} bytes\n\n", image_metadata.size_bytes));
                    }
                }
            }

            // Add the text content
            if let Some(text) = &context.text {
                context_content.push_str(text);
            }
            context_content.push_str("\n\n");

            // Add a separator between chunks, except for the last one
            if index < self.context.len() - 1 {
                context_content.push_str("---\n\n");
            }
        }

        context_content
    }
}

impl Message<RetrieveContext> for Retriever {
    type Response = RetrievedContext;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: &RetrieveContext,
    ) -> Result<RetrievedContext, RetrieverError> {
        let Some(embeddings_id) = &self.embeddings_id else {
            return Err(RetrieverError::EmbeddingsIdNotFound);
        };
        let Some(rerank_id) = &self.rerank_id else {
            return Err(RetrieverError::RerankIdNotFound);
        };

        match &message.query {
            QueryType::Text(text) => {
                info!("Fetching context for query: {}", text);
                let embeddings_req = embeddings::TopK {
                    query: embeddings::Query::Text(text.clone()),
                    k: message.limit * 2,
                    threshold: message.threshold,
                    tag: Some(self.tag.clone().to_string()),
                };

                info!("Searching for similarities");
                let start = std::time::Instant::now();
                let similarities = match ctx
                    .send::<Embeddings, embeddings::TopK>(
                        embeddings_req,
                        embeddings_id,
                        SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
                    )
                    .await
                {
                    Ok(result) => result,
                    Err(e) => {
                        error!("Failed to get similarities: {}", e);
                        return Err(RetrieverError::ComputingSimilarity(e.to_string()));
                    }
                };
                info!("Similarities: {} in {:?}", similarities.len(), start.elapsed());

                // Only rerank if we have text content
                let context = if similarities.iter().any(|s| s.text.is_some()) {
                    // Rank the embeddings only for text content
                    info!("Ranking similarity texts");
                    let start = std::time::Instant::now();
                    let texts: Vec<String> = similarities.iter().filter_map(|s| s.text.clone()).collect();

                    let rerank_req = RankTexts { query: text.clone(), texts };

                    let ranked_texts = match ctx
                        .send::<Rerank, RankTexts>(
                            rerank_req,
                            rerank_id,
                            SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
                        )
                        .await
                    {
                        Ok(result) => result,
                        Err(e) => {
                            error!("Failed to rank texts: {}", e);
                            return Err(RetrieverError::ComputingRerank(e.to_string()));
                        }
                    };
                    info!("Ranked texts: {} in {:?}", ranked_texts.texts.len(), start.elapsed());

                    ranked_texts
                        .texts
                        .into_iter()
                        .map(|t| Context {
                            text: similarities[t.index].text.clone(),
                            metadata: similarities[t.index]
                                .metadata
                                .as_ref()
                                .and_then(|m| serde_json::from_value(m.clone()).ok()),
                        })
                        .take(message.limit)
                        .collect()
                } else {
                    // For non-text content (images, etc.), use similarities directly
                    similarities
                        .into_iter()
                        .map(|s| Context {
                            text: s.text,
                            metadata: s.metadata.and_then(|m| serde_json::from_value(m).ok()),
                        })
                        .take(message.limit)
                        .collect()
                };

                Ok(RetrievedContext { context })
            }
            _ => unimplemented!("Only Text queries are currently supported"),
        }
    }
}

#[derive(bon::Builder, Debug, Serialize, Deserialize)]
pub struct Retriever {
    #[builder(default)]
    pub embeddings: Embeddings,
    #[builder(default)]
    pub rerank: Rerank,
    #[builder(default = DEFAULT_RETRIEVER_TAG.into())]
    pub tag: Cow<'static, str>,
    embeddings_id: Option<ActorId>,
    rerank_id: Option<ActorId>,
    #[serde(skip)]
    embeddings_handle: Option<tokio::task::JoinHandle<()>>,
    #[serde(skip)]
    rerank_handle: Option<tokio::task::JoinHandle<()>>,
}

impl Default for Retriever {
    fn default() -> Self {
        Self {
            embeddings: Embeddings::default(),
            rerank: Rerank::default(),
            tag: DEFAULT_RETRIEVER_TAG.into(),
            embeddings_id: None,
            rerank_id: None,
            embeddings_handle: None,
            rerank_handle: None,
        }
    }
}

impl Actor for Retriever {
    type Error = RetrieverError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), RetrieverError> {
        self.init(ctx).await?;

        // Start the message stream
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(input) = frame.is::<RetrieveContext>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            }
        }
        Ok(())
    }
}

impl Retriever {
    pub async fn init(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), RetrieverError> {
        let self_id = ctx.id().clone();
        let embeddings_id = ActorId::of::<Embeddings>(format!("{}/embeddings", self_id.name()));
        let rerank_id = ActorId::of::<Rerank>(format!("{}/rerank", self_id.name()));

        self.embeddings_id = Some(embeddings_id.clone());
        self.rerank_id = Some(rerank_id.clone());

        // Spawn the embeddings actor
        let (mut embeddings_ctx, mut embeddings_actor) = Actor::spawn(
            ctx.engine().clone(),
            embeddings_id.clone(),
            self.embeddings.clone(),
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;

        // Spawn the rerank actor
        let (mut rerank_ctx, mut rerank_actor) = Actor::spawn(
            ctx.engine().clone(),
            rerank_id.clone(),
            self.rerank.clone(),
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;

        // Start the embeddings actor
        let embeddings_handle = tokio::spawn(async move {
            if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
                error!("Embeddings actor error: {}", e);
            }
        });

        // Start the rerank actor
        let rerank_handle = tokio::spawn(async move {
            if let Err(e) = rerank_actor.start(&mut rerank_ctx).await {
                error!("Rerank actor error: {}", e);
            }
        });

        self.embeddings_handle = Some(embeddings_handle);
        self.rerank_handle = Some(rerank_handle);

        info!("Retriever ready");

        Ok(())
    }
}
