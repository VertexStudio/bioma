use bioma_actor::prelude::*;
use derive_more::Display;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankTexts {
    pub query: String,
    pub texts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedText {
    pub index: usize,
    pub score: f32,
}

impl Message<RankTexts> for Rerank {
    type Response = Vec<RankedText>;

    async fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        rank_texts: &RankTexts,
    ) -> Result<Vec<RankedText>, RerankError> {
        if rank_texts.texts.is_empty() {
            warn!("No texts to rerank");
            return Ok(vec![]);
        }

        let Some((rerank_tx, _)) = self.rerank_task.as_ref() else {
            return Err(RerankError::RerankNotInitialized);
        };

        let (tx, rx) = oneshot::channel();
        rerank_tx
            .send(RerankRequest { sender: tx, query: rank_texts.query.clone(), texts: rank_texts.texts.clone() })
            .await?;

        Ok(rx.await??)
    }
}

pub struct RerankRequest {
    sender: oneshot::Sender<Result<Vec<RankedText>, fastembed::Error>>,
    query: String,
    texts: Vec<String>,
}

#[derive(bon::Builder, Debug, Serialize, Deserialize)]
pub struct Rerank {
    #[builder(default = Model::JINARerankerV2BaseMultiligual)]
    pub model: Model,
    #[serde(skip)]
    rerank_task: Option<(mpsc::Sender<RerankRequest>, JoinHandle<Result<(), fastembed::Error>>)>,
}

impl Default for Rerank {
    fn default() -> Self {
        Self { model: Model::JINARerankerV2BaseMultiligual, rerank_task: None }
    }
}

impl Clone for Rerank {
    fn clone(&self) -> Self {
        Self { model: self.model.clone(), rerank_task: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Display)]
pub enum Model {
    JINARerankerV2BaseMultiligual,
}

fn get_fastembed_model(model: &Model) -> fastembed::RerankerModel {
    match model {
        Model::JINARerankerV2BaseMultiligual => fastembed::RerankerModel::JINARerankerV2BaseMultiligual,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: Model,
    pub dim: usize,
    pub description: String,
    pub model_code: String,
    pub model_file: String,
}

impl Actor for Rerank {
    type Error = RerankError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), RerankError> {
        info!("{} Started", ctx.id());

        // Spawn the rerank task
        let model = self.model.clone();
        let (tx, mut rx) = mpsc::channel::<RerankRequest>(100);
        let rerank_task: JoinHandle<Result<(), fastembed::Error>> = tokio::task::spawn_blocking(move || {
            let reranker = fastembed::TextRerank::try_new(
                fastembed::RerankInitOptions::new(get_fastembed_model(&model)).with_show_download_progress(true),
            )?;
            while let Some(request) = rx.blocking_recv() {
                let texts = request.texts.iter().map(|text| text).collect::<Vec<&String>>();
                let results = reranker.rerank(&request.query, texts, false, None)?;
                let ranked_texts =
                    results.into_iter().map(|result| RankedText { index: result.index, score: result.score }).collect();
                let _ = request.sender.send(Ok(ranked_texts));
            }
            Ok(())
        });

        self.rerank_task = Some((tx, rerank_task));

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
