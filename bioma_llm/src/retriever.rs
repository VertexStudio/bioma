use crate::embeddings::{self, Embeddings, EmbeddingsError};
use crate::indexer::{ChunkMetadata, Source};
use crate::rerank::{RankTexts, Rerank, RerankError};
use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrieveContext {
    pub query: String,
    pub limit: usize,
    pub threshold: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context {
    pub text: String,
    pub metadata: Option<ChunkMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievedContext {
    pub context: Vec<Context>,
}

impl RetrievedContext {
    pub fn to_markdown(&self) -> String {
        let mut context_content = String::new();

        for (index, context) in self.context.iter().enumerate() {
            // Add source information
            if let Some(metadata) = &context.metadata {
                context_content.push_str("Source: ");
                match &metadata.source {
                    Source::File(path) => context_content.push_str(&format!("File - {}\n", path.display())),
                    Source::Url(url) => context_content.push_str(&format!("URL - {}\n", url)),
                }
                context_content.push_str(&format!("Chunk: {}\n\n", metadata.chunk_number));
            }

            // Add the text content
            context_content.push_str(&context.text);
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

        info!("Fetching context for query: {}", message.query);
        let embeddings_req = embeddings::TopK {
            query: embeddings::Query::Text(message.query.clone()),
            k: message.limit * 2,
            threshold: message.threshold,
            tag: Some(self.tag.clone()),
        };
        info!("Searching for texts similarities");
        let similarities =
            match ctx.send::<Embeddings, embeddings::TopK>(embeddings_req, embeddings_id, SendOptions::default()).await
            {
                Ok(result) => result,
                Err(e) => {
                    error!("Failed to get similarities: {}", e);
                    return Err(RetrieverError::ComputingSimilarity(e.to_string()));
                }
            };
        // Rank the embeddings
        info!("Ranking similarity texts");
        let rerank_req = RankTexts {
            query: message.query.clone(),
            texts: similarities.iter().map(|s| s.text.clone()).collect(),
            raw_scores: false,
        };
        let ranked_texts = match ctx.send::<Rerank, RankTexts>(rerank_req, rerank_id, SendOptions::default()).await {
            Ok(result) => result,
            Err(e) => {
                error!("Failed to rank texts: {}", e);
                return Err(RetrieverError::ComputingRerank(e.to_string()));
            }
        };
        // Get the context from the ranked texts
        info!("Getting context from ranked texts");
        let context = if ranked_texts.len() > 0 {
            ranked_texts
                .iter()
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
            vec![Context { text: "No context found".to_string(), metadata: None }]
        };
        Ok(RetrievedContext { context })
    }
}

#[derive(bon::Builder, Debug, Serialize, Deserialize)]
pub struct Retriever {
    pub embeddings: Embeddings,
    pub rerank: Rerank,
    pub tag: String,
    embeddings_id: Option<ActorId>,
    rerank_id: Option<ActorId>,
}

impl Default for Retriever {
    fn default() -> Self {
        Self {
            embeddings: Embeddings::default(),
            rerank: Rerank::default(),
            tag: "indexer_content".to_string(),
            embeddings_id: None,
            rerank_id: None,
        }
    }
}

impl Actor for Retriever {
    type Error = RetrieverError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), RetrieverError> {
        let self_id = ctx.id().clone();
        let embeddings_id = ActorId::of::<Embeddings>(format!("{}/embeddings", self_id.name()));
        let rerank_id = ActorId::of::<Rerank>(format!("{}/rerank", self_id.name()));

        self.embeddings_id = Some(embeddings_id.clone());
        self.rerank_id = Some(rerank_id.clone());

        let (mut embeddings_ctx, mut embeddings_actor) = Actor::spawn(
            ctx.engine().clone(),
            embeddings_id.clone(),
            self.embeddings.clone(),
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;
        let (mut rerank_ctx, mut rerank_actor) = Actor::spawn(
            ctx.engine().clone(),
            rerank_id.clone(),
            self.rerank.clone(),
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;
        let embeddings_handle = tokio::spawn(async move {
            if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
                error!("Embeddings actor error: {}", e);
            }
        });
        let rerank_handle = tokio::spawn(async move {
            if let Err(e) = rerank_actor.start(&mut rerank_ctx).await {
                error!("Rerank actor error: {}", e);
            }
        });
        info!("Retriever ready");

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
        embeddings_handle.abort();
        rerank_handle.abort();
        Ok(())
    }
}
