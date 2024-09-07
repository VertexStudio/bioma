use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use tracing::{error, info};
use url::Url;

/// Enumerates the types of errors that can occur in LLM
#[derive(thiserror::Error, Debug)]
pub enum RerankError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Rerank requesterror: {0}")]
    RerankRequest(#[from] reqwest::Error),
    #[error("Rerank response error: {0}")]
    RerankResponse(String),
}

impl ActorError for RerankError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankTexts {
    pub query: String,
    pub texts: Vec<String>,
    pub raw_scores: bool,
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
        let client = reqwest::Client::new();
        let res = client.post(self.url.clone()).json(&rank_texts).send().await?;
        if !res.status().is_success() {
            return Err(RerankError::RerankResponse(res.text().await?));
        }
        let ranked_texts: Vec<RankedText> = res.json().await?;
        Ok(ranked_texts)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rerank {
    pub url: Url,
}

impl Actor for Rerank {
    type Error = RerankError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), RerankError> {
        info!("{} Started", ctx.id());
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
