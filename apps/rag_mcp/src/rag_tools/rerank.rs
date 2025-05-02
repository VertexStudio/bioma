use bioma_actor::{Actor, ActorId, Engine, Relay, SendOptions, SpawnOptions, SystemActorError};
use bioma_mcp::{
    schema::CallToolResult,
    server::RequestContext,
    tools::{ToolDef, ToolError},
};
use bioma_rag::prelude::{RankTexts, Rerank};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::time::Duration;
use tracing::error;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TextToRank {
    #[schemars(description = "Text to be ranked")]
    text: String,

    #[schemars(description = "Optional ID to identify this text")]
    id: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct RerankArgs {
    #[schemars(description = "Query text to compare against")]
    query: String,

    #[schemars(description = "Array of texts to rank by relevance to the query")]
    texts: Vec<TextToRank>,

    #[schemars(description = "Model to use for reranking")]
    model: Option<String>,

    #[schemars(description = "Return only top N results")]
    top_n: Option<usize>,
}

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
    type Args = RerankArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        let relay_id = ActorId::of::<Relay>("/rag/rerank/relay");

        let (relay_ctx, _) = Actor::spawn(self.engine.clone(), relay_id, Relay, SpawnOptions::default())
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to spawn relay: {}", e)))?;

        // Convert RerankArgs to RankTexts
        let texts = args.texts.into_iter().map(|t| t.text).collect();

        let rank_texts = RankTexts::builder().query(args.query).texts(texts).raw_scores(true).return_text(true).build();

        let response = relay_ctx
            .send_and_wait_reply::<Rerank, RankTexts>(
                rank_texts,
                &self.id,
                SendOptions::builder().timeout(Duration::from_secs(200)).build(),
            )
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to rerank texts: {}", e)))?;

        let response_value = serde_json::to_value(response)
            .map_err(|e| ToolError::Execution(format!("Failed to serialize response: {}", e)))?;

        Ok(CallToolResult { meta: None, content: vec![response_value], is_error: None })
    }
}
