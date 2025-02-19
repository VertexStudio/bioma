// use crate::ORT_EXIT_MUTEX;
use bioma_actor::prelude::*;
use bon::Builder;
use derive_more::{Deref, Display};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Weak};
use tokio::sync::Mutex;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

// Max number of tokens to be processed for each input
const DEFAULT_MAX_LENGTH: usize = 512;

/// Enumerates the types of errors that can occur in LLM
#[derive(thiserror::Error, Debug)]
pub enum RerankError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Fastembed error: {0}")]
    Fastembed(#[from] fastembed::Error),
    #[error("Rerank not initialized")]
    RerankNotInitialized,
    #[error("Error sending rerank request: {0}")]
    SendRerankRequest(#[from] mpsc::error::SendError<RerankRequest>),
    #[error("Error receiving rerank response: {0}")]
    RecvRerankResponse(#[from] oneshot::error::RecvError),
}

impl ActorError for RerankError {}

#[derive(Builder, Debug, Clone, Serialize, Deserialize)]
pub struct RankTexts {
    /// The query text to compare against the corpus of texts
    ///
    /// # Example
    /// ```
    /// let query = "What is Deep Learning?";
    /// ```
    pub query: String,

    /// Whether to return raw similarity scores (true) or normalized probabilities (false)
    ///
    /// When false, scores are normalized using softmax to produce probabilities that sum to 1.0
    #[serde(default = "default_raw_scores")]
    #[builder(default = default_raw_scores())]
    pub raw_scores: bool,

    /// Whether to include the original text in the response
    ///
    /// When true, the response will include the full text for each result.
    /// When false, only indices and scores are returned.
    #[serde(default = "default_return_text")]
    #[builder(default = default_return_text())]
    pub return_text: bool,

    /// The corpus of texts to rank by similarity to the query
    pub texts: Vec<String>,

    /// Whether to truncate texts that exceed the model's maximum length
    #[serde(default = "default_truncate")]
    #[builder(default = default_truncate())]
    pub truncate: bool,

    /// The direction to truncate texts from when truncation is enabled
    ///
    /// Can be either "left" or "right". Defaults to truncating from the right.
    #[serde(default = "default_truncation_direction")]
    #[builder(default = default_truncation_direction())]
    pub truncation_direction: TruncationDirection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// #[serde(rename_all = "lowercase")]
pub enum TruncationDirection {
    Left,
    Right,
}

pub fn default_raw_scores() -> bool {
    false
}

pub fn default_return_text() -> bool {
    false
}

pub fn default_truncate() -> bool {
    false
}

pub fn default_truncation_direction() -> TruncationDirection {
    TruncationDirection::Right
}

#[derive(utoipa::ToSchema, Debug, Clone, Serialize, Deserialize)]
pub struct RankedText {
    pub index: usize,
    pub score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(utoipa::ToSchema, Debug, Clone, Serialize, Deserialize)]
pub struct RankedTexts {
    pub texts: Vec<RankedText>,
}

impl Message<RankTexts> for Rerank {
    type Response = RankedTexts;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, rank_texts: &RankTexts) -> Result<(), RerankError> {
        if rank_texts.texts.is_empty() {
            warn!("No texts to rerank");
            ctx.reply(RankedTexts { texts: vec![] }).await?;
            return Ok(());
        }

        let Some(rerank_tx) = self.rerank_tx.as_ref() else {
            return Err(RerankError::RerankNotInitialized);
        };

        let (tx, rx) = oneshot::channel();
        rerank_tx.send(RerankRequest { sender: tx, message: rank_texts.clone() }).await?;

        ctx.reply(rx.await??).await?;
        Ok(())
    }
}

pub struct RerankRequest {
    sender: oneshot::Sender<Result<RankedTexts, fastembed::Error>>,
    message: RankTexts,
}

lazy_static! {
    static ref SHARED_RERANK: Arc<Mutex<Weak<SharedRerank>>> = Arc::new(Mutex::new(Weak::new()));
}

struct SharedRerank {
    rerank_tx: mpsc::Sender<RerankRequest>,
}

#[derive(Deref)]
struct StrongSharedRerank(Arc<SharedRerank>);

impl std::fmt::Debug for StrongSharedRerank {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StrongSharedRerank")
    }
}

#[derive(bon::Builder, Debug, Serialize, Deserialize)]
pub struct Rerank {
    #[builder(default = Model::BGERerankerV2M3)]
    pub model: Model,
    #[serde(skip)]
    rerank_tx: Option<mpsc::Sender<RerankRequest>>,
    #[serde(skip)]
    shared_rerank: Option<StrongSharedRerank>,
}

impl Default for Rerank {
    fn default() -> Self {
        Rerank::builder().build()
    }
}

impl Clone for Rerank {
    fn clone(&self) -> Self {
        Self { model: self.model.clone(), rerank_tx: None, shared_rerank: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Display)]
pub enum Model {
    BGERerankerBase,
    BGERerankerV2M3,
}

fn get_fastembed_model(model: &Model) -> fastembed::RerankerModel {
    match model {
        Model::BGERerankerBase => fastembed::RerankerModel::BGERerankerBase,
        Model::BGERerankerV2M3 => fastembed::RerankerModel::BGERerankerV2M3,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: Model,
    pub dim: usize,
    pub description: String,
    pub model_code: String,
}

impl Actor for Rerank {
    type Error = RerankError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), RerankError> {
        info!("{} Started", ctx.id());

        self.init(ctx).await?;

        // Start the message stream
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(rank_texts) = frame.is::<RankTexts>() {
                let response = self.reply(ctx, &rank_texts, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            }
        }
        info!("{} Finished", ctx.id());
        Ok(())
    }
}

impl Rerank {
    pub async fn init(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), RerankError> {
        info!("{} Started", ctx.id());

        // Manage a shared rerank task
        let shared_rerank = {
            let mut weak_ref = SHARED_RERANK.lock().await;
            if let Some(strong_ref) = weak_ref.upgrade() {
                // Return the existing shared rerank
                Some(strong_ref)
            } else {
                // Create a new shared rerank
                let model = self.model.clone();
                let ctx_id = ctx.id().clone();
                let cache_dir = ctx.engine().huggingface_cache_dir().clone();
                let (rerank_tx, mut rerank_rx) = mpsc::channel::<RerankRequest>(100);
                let _rerank_task: JoinHandle<Result<(), fastembed::Error>> = tokio::task::spawn_blocking(move || {
                    // Get the reranker model
                    let model = get_fastembed_model(&model);

                    let mut options = fastembed::RerankInitOptions::new(model)
                        .with_cache_dir(cache_dir)
                        .with_max_length(DEFAULT_MAX_LENGTH);

                    #[cfg(target_os = "macos")]
                    {
                        options = options.with_execution_providers(vec![
                            ort::execution_providers::CoreMLExecutionProvider::default().build(),
                        ]);
                    }

                    #[cfg(target_os = "linux")]
                    {
                        options = options.with_execution_providers(vec![
                            ort::execution_providers::CUDAExecutionProvider::default().build(),
                        ]);
                    }

                    let reranker = fastembed::TextRerank::try_new(options)?;

                    while let Some(request) = rerank_rx.blocking_recv() {
                        let start = std::time::Instant::now();
                        let texts = request.message.texts.iter().map(|text| text).collect::<Vec<&String>>();
                        match reranker.rerank(&request.message.query, texts, false, None) {
                            Ok(results) => {
                                // compute average text length
                                let avg_text_len =
                                    request.message.texts.iter().map(|text| text.len() as f32).sum::<f32>()
                                        / request.message.texts.len() as f32;
                                info!(
                                    "Ranked {} texts (avg. {:.1} chars) in {:?}",
                                    request.message.texts.len(),
                                    avg_text_len,
                                    start.elapsed()
                                );

                                let mut ranked_texts = RankedTexts {
                                    texts: results
                                        .into_iter()
                                        .map(|result| RankedText {
                                            index: result.index,
                                            score: result.score,
                                            text: match request.message.return_text {
                                                true => Some(request.message.texts[result.index].clone()),
                                                false => None,
                                            },
                                        })
                                        .collect(),
                                };

                                if !request.message.raw_scores {
                                    // Apply softmax normalization to scores
                                    let max_score =
                                        ranked_texts.texts.iter().map(|t| t.score).fold(f32::NEG_INFINITY, f32::max);
                                    let sum: f32 = ranked_texts.texts.iter().map(|t| (t.score - max_score).exp()).sum();

                                    for text in &mut ranked_texts.texts {
                                        text.score = ((text.score - max_score).exp()) / sum;
                                    }
                                }

                                let _ = request.sender.send(Ok(ranked_texts));
                            }
                            Err(err) => {
                                error!("Rerank failed: {}", err);
                                let _ = request.sender.send(Err(err));
                            }
                        }
                    }

                    info!("{} rerank finished", ctx_id);
                    drop(reranker);

                    Ok(())
                });

                // Store the shared rerank
                let shared_rerank = Arc::new(SharedRerank { rerank_tx });
                *weak_ref = Arc::downgrade(&shared_rerank);
                Some(shared_rerank)
            }
        };

        self.shared_rerank = shared_rerank.map(StrongSharedRerank);
        self.rerank_tx = self.shared_rerank.as_ref().map(|sr| sr.rerank_tx.clone());

        Ok(())
    }
}
