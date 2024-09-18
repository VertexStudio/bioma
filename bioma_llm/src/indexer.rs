use crate::embeddings::{self, Embeddings, EmbeddingsError, GenerateEmbeddings};
use crate::rerank::{self, RankTexts, Rerank, RerankError};
use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use tracing::{debug, error, info, warn};
use url::Url;

#[derive(thiserror::Error, Debug)]
pub enum IndexerError {
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
}

impl ActorError for IndexerError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexGlobs {
    pub globs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexGlobsResponse {
    pub indexed: usize,
    pub cached: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchContext {
    pub query: String,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchContextResponse {
    pub context: Vec<String>,
}

impl Message<FetchContext> for Indexer {
    type Response = FetchContextResponse;
    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: &FetchContext,
    ) -> Result<FetchContextResponse, IndexerError> {
        info!("Fetching context for query: {}", message.query);
        let embeddings_req = embeddings::TopK {
            query: embeddings::Query::Text(message.query.clone()),
            k: message.limit * 2,
            threshold: 0.0,
            tag: Some("indexer_content".to_string()),
        };
        info!("Searching for texts similarities");
        let similarities = match ctx
            .send::<Embeddings, embeddings::TopK>(embeddings_req, &self.embeddings_actor, SendOptions::default())
            .await
        {
            Ok(result) => result,
            Err(e) => {
                error!("Failed to get similarities: {}", e);
                return Err(IndexerError::ComputingSimilarity(e.to_string()));
            }
        };
        // Rank the embeddings
        info!("Ranking similarity texts");
        let rerank_req = RankTexts {
            query: message.query.clone(),
            texts: similarities.iter().map(|s| s.text.clone()).collect(),
            raw_scores: false,
        };
        let ranked_texts =
            match ctx.send::<Rerank, RankTexts>(rerank_req, &self.rerank_actor, SendOptions::default()).await {
                Ok(result) => result,
                Err(e) => {
                    error!("Failed to rank texts: {}", e);
                    return Err(IndexerError::ComputingRerank(e.to_string()));
                }
            };
        // Get the context from the ranked texts
        info!("Getting context from ranked texts");
        let context = if ranked_texts.len() > 0 {
            ranked_texts.iter().map(|t| similarities[t.index].text.clone()).take(message.limit).collect()
        } else {
            vec!["No context found".to_string()]
        };
        Ok(FetchContextResponse { context })
    }
}

impl Message<IndexGlobs> for Indexer {
    type Response = IndexGlobsResponse;
    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: &IndexGlobs,
    ) -> Result<IndexGlobsResponse, IndexerError> {
        let mut indexed = 0;
        let mut cached = 0;
        for glob in message.globs.iter() {
            info!("Indexing glob: {}", &glob);
            let task_glob = glob.clone();
            let paths = tokio::task::spawn_blocking(move || glob::glob(&task_glob)).await;
            let Ok(Ok(paths)) = paths else {
                warn!("Skipping glob: {}", &glob);
                continue;
            };
            for path in paths {
                let Ok(path) = path else {
                    warn!("Skipping path: {:?}", path);
                    continue;
                };
                if self.cache.contains(&path) {
                    debug!("Path already indexed: {}", path.display());
                    cached += 1;
                    continue;
                }
                self.cache.insert(path.clone());
                info!("Indexing path: {}", &path.display());
                let content = tokio::fs::read_to_string(path).await?;
                ctx.send::<Embeddings, GenerateEmbeddings>(
                    GenerateEmbeddings { texts: vec![content], tag: Some("indexer_content".to_string()) },
                    &self.embeddings_actor,
                    SendOptions::default(),
                )
                .await?;
                indexed += 1;
            }
        }
        if indexed > 0 {
            self.save(ctx).await?;
        }
        info!("Indexed {} paths, cached {} paths", indexed, cached);
        Ok(IndexGlobsResponse { indexed, cached })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Indexer {
    embeddings_actor: ActorId,
    rerank_actor: ActorId,
    cache: HashSet<PathBuf>,
}

impl Default for Indexer {
    fn default() -> Self {
        Self {
            embeddings_actor: ActorId::of::<Embeddings>("/indexer/embeddings"),
            rerank_actor: ActorId::of::<Rerank>("/indexer/rerank"),
            cache: HashSet::new(),
        }
    }
}

impl Actor for Indexer {
    type Error = IndexerError;
    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), IndexerError> {
        let (mut embeddings_ctx, mut embeddings_actor) = Actor::spawn(
            ctx.engine().clone(),
            self.embeddings_actor.clone(),
            Embeddings { model_name: "nomic-embed-text".to_string(), ..Default::default() },
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;
        let (mut rerank_ctx, mut rerank_actor) = Actor::spawn(
            ctx.engine().clone(),
            self.rerank_actor.clone(),
            Rerank { url: Url::parse("http://localhost:9124/rerank").unwrap() },
            SpawnOptions::builder().exists(SpawnExistsOptions::Restore).build(),
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
        info!("Indexer ready");
        // Start the message stream
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(input) = frame.is::<IndexGlobs>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            } else if let Some(input) = frame.is::<FetchContext>() {
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
