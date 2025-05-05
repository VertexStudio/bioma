use anyhow::Error;
use bioma_actor::{Actor, ActorId, Engine, Relay, SendOptions, SpawnOptions, SystemActorError};
use bioma_mcp::{schema::CallToolResult, server::RequestContext, tools::ToolDef};
use bioma_rag::prelude::{RankTexts as RankTextsArgs, Rerank};
use serde::Serialize;
use std::{borrow::Cow, time::Duration};
use tracing::error;

#[derive(Serialize)]
pub struct RerankTool {
    #[serde(skip_serializing)]
    id: ActorId,
    #[serde(skip_serializing)]
    engine: Engine,
}

impl RerankTool {
    pub async fn new(engine: &Engine) -> Result<Self, SystemActorError> {
        let id = ActorId::of::<Rerank>("/rag/rerank");

        let (mut rerank_ctx, mut rerank_actor) =
            Actor::spawn(engine.clone(), id.clone(), Rerank::default(), SpawnOptions::default()).await.map_err(
                |e| SystemActorError::LiveStream(Cow::Owned(format!("Failed to spawn rerank actor: {}", e))),
            )?;

        tokio::spawn(async move {
            if let Err(e) = rerank_actor.start(&mut rerank_ctx).await {
                error!("Rerank actor error: {}", e);
            }
        });

        Ok(Self { id, engine: engine.clone() })
    }
}

impl ToolDef for RerankTool {
    const NAME: &'static str = "rerank";
    const DESCRIPTION: &'static str = "Reranks a list of texts based on their relevance to a query";
    type Args = RankTextsArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, Error> {
        let relay_id = ActorId::of::<Relay>("/rag/rerank/relay");

        let (relay_ctx, _) = Actor::spawn(self.engine.clone(), relay_id, Relay, SpawnOptions::default()).await?;

        let response = relay_ctx
            .send_and_wait_reply::<Rerank, RankTextsArgs>(
                args,
                &self.id,
                SendOptions::builder().timeout(Duration::from_secs(200)).build(),
            )
            .await?;

        let response_value = serde_json::to_value(response)?;

        Ok(CallToolResult { meta: None, content: vec![response_value], is_error: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bioma_mcp::server::RequestContext;
    use bioma_rag::{prelude::RankTexts as RankTextsArgs, rerank::RankedTexts};

    #[tokio::test]
    async fn rerank_texts() {
        let engine = Engine::test().await.unwrap();
        let rerank_tool = RerankTool::new(&engine).await.unwrap();

        let raw = rerank_tool
            .call(
                RankTextsArgs::builder()
                    .query("memory safety".into())
                    .texts(vec![
                        "Rust guarantees memory-safety with its borrow-checker".into(),
                        "Java is a garbage-collected language".into(),
                        "C lets you do manual memory management".into(),
                    ])
                    .return_text(true)
                    .build(),
                RequestContext::default(),
            )
            .await
            .expect("rerank should succeed");

        let ranked: RankedTexts = serde_json::from_value(raw.content[0].clone()).unwrap();

        println!("ranked: {:?}", ranked);

        assert_eq!(ranked.texts.len(), 3);

        let scores: Vec<f32> = ranked.texts.iter().map(|r| r.score).collect();
        assert!(scores.windows(2).all(|w| w[0] >= w[1]), "scores should be sorted descending");

        let best = &ranked.texts[0];
        let best_text = best.text.as_deref().unwrap_or_default().to_lowercase();
        assert!(best_text.contains("rust") || best_text.contains("memory"), "best match should relate to the query");

        assert!(ranked.texts.iter().all(|r| r.index < 3));
    }
}
