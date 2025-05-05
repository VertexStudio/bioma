use bioma_actor::{Actor, ActorId, Engine, Relay, SendOptions, SpawnExistsOptions, SpawnOptions, SystemActorError};
use bioma_mcp::{
    schema::CallToolResult,
    server::RequestContext,
    tools::{ToolDef, ToolError},
};
use bioma_rag::prelude::{Index as IndexArgs, Indexer};
use serde::Serialize;
use std::borrow::Cow;
use std::time::Duration;
use tracing::error;

#[derive(Serialize)]
pub struct IndexTool {
    #[serde(skip_serializing)]
    id: ActorId,
    #[serde(skip_serializing)]
    engine: Engine,
}

impl IndexTool {
    pub async fn new(engine: &Engine) -> Result<Self, SystemActorError> {
        let id = ActorId::of::<Indexer>("/rag/indexer");

        let (mut indexer_ctx, mut indexer_actor) = Actor::spawn(
            engine.clone(),
            id.clone(),
            Indexer::default(),
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await
        .map_err(|e| SystemActorError::LiveStream(Cow::Owned(format!("Failed to spawn indexer: {}", e))))?;

        tokio::spawn(async move {
            if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
                error!("Indexer actor error: {}", e);
            }
        });

        Ok(Self { id, engine: engine.clone() })
    }
}

impl ToolDef for IndexTool {
    const NAME: &'static str = "index";
    const DESCRIPTION: &'static str = "Indexes content from text or URLs for future retrieval and RAG";
    type Args = IndexArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        let relay_id = ActorId::of::<Relay>("/rag/indexer/relay");

        let (relay_ctx, _) = Actor::spawn(self.engine.clone(), relay_id, Relay, SpawnOptions::default())
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to spawn relay: {}", e)))?;

        let response = relay_ctx
            .send_and_wait_reply::<Indexer, IndexArgs>(
                args,
                &self.id,
                SendOptions::builder().timeout(Duration::from_secs(600)).build(),
            )
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to index content: {}", e)))?;

        let response_value = serde_json::to_value(response)
            .map_err(|e| ToolError::Execution(format!("Failed to serialize response: {}", e)))?;

        Ok(CallToolResult { meta: None, content: vec![response_value], is_error: None })
    }
}
