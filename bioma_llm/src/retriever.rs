use crate::embeddings::{self, Embeddings, EmbeddingsError};
use crate::indexer::{ContentSource, Metadata};
use crate::rerank::{RankTexts, Rerank, RerankError};
use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

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
    /// The query to search for
    #[serde(flatten)]
    pub query: RetrieveQuery,
    /// The number of contexts to return
    #[builder(default = DEFAULT_RETRIEVER_LIMIT)]
    #[serde(default = "default_retriever_limit")]
    pub limit: usize,
    /// The threshold for the similarity score
    #[builder(default = DEFAULT_RETRIEVER_THRESHOLD)]
    #[serde(default = "default_retriever_threshold")]
    pub threshold: f32,
    /// A source regex pattern to filter the search
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "query")]
pub enum RetrieveQuery {
    Text(String),
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
    pub source: Option<ContentSource>,
    pub metadata: Option<Metadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievedContext {
    pub context: Vec<Context>,
}

impl RetrievedContext {
    pub fn to_markdown(&self) -> String {
        let mut context_content = String::new();

        for (index, context) in self.context.iter().enumerate() {
            if let Some(source) = &context.source {
                context_content.push_str(&format!("Source: {}\n", source.source));
                context_content.push_str(&format!("URI: {}\n", source.uri));
            }

            match &context.metadata {
                Some(Metadata::Text(text_metadata)) => {
                    context_content.push_str(&format!("Chunk: {}\n", text_metadata.chunk_number))
                }
                Some(Metadata::Image(image_metadata)) => {
                    context_content.push_str(&format!("Image: {}\n", image_metadata.format))
                }
                None => (),
            }

            if let Some(text) = &context.text {
                context_content.push_str(text);
            }
            context_content.push_str("\n\n");

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
            RetrieveQuery::Text(text) => {
                info!("Fetching context for query: {}", text);
                let embeddings_req = embeddings::TopK {
                    query: embeddings::Query::Text(text.clone()),
                    k: message.limit * 2,
                    threshold: message.threshold,
                    source: message.source.clone(),
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

                // Separate text and image content based on ContentType
                let (text_similarities, image_similarities): (Vec<_>, Vec<_>) =
                    similarities.into_iter().map(|s| (s.clone(), s.similarity)).partition(|(s, _)| {
                        s.metadata
                            .as_ref()
                            .and_then(|m| serde_json::from_value::<Metadata>(m.clone()).ok())
                            .map_or(false, |metadata| matches!(metadata, Metadata::Text(_)))
                    });

                // Process text content with reranking
                let mut ranked_contexts = if !text_similarities.is_empty() {
                    let texts: Vec<String> = text_similarities.iter().filter_map(|(s, _)| s.text.clone()).collect();

                    let rerank_req = RankTexts { query: text.clone(), texts };
                    let ranked_texts = ctx
                        .send::<Rerank, RankTexts>(
                            rerank_req,
                            rerank_id,
                            SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
                        )
                        .await?;

                    // Create contexts with rerank scores
                    ranked_texts
                        .texts
                        .into_iter()
                        .map(|t| {
                            (
                                Context {
                                    text: text_similarities[t.index].0.text.clone(),
                                    source: None,
                                    metadata: text_similarities[t.index]
                                        .0
                                        .metadata
                                        .as_ref()
                                        .and_then(|m| serde_json::from_value(m.clone()).ok()),
                                },
                                t.score,
                            )
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };

                // Add image contexts with their similarity scores
                ranked_contexts.extend(image_similarities.into_iter().map(|(s, score)| {
                    (
                        Context {
                            text: s.text,
                            source: None,
                            metadata: s.metadata.and_then(|m| serde_json::from_value(m).ok()),
                        },
                        score,
                    )
                }));

                // Sort all contexts by score in descending order
                ranked_contexts.sort_by(|(_, a_score), (_, b_score)| {
                    b_score.partial_cmp(a_score).unwrap_or(std::cmp::Ordering::Equal)
                });

                // Take only the contexts, limited by the requested amount
                let contexts = ranked_contexts.into_iter().map(|(context, _)| context).take(message.limit).collect();

                Ok(RetrievedContext { context: contexts })
            }
        }
    }
}

#[derive(bon::Builder, Debug, Serialize, Deserialize)]
pub struct Retriever {
    #[builder(default)]
    pub embeddings: Embeddings,
    #[builder(default)]
    pub rerank: Rerank,
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
