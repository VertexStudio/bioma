use bioma_actor::{Actor, ActorId, Engine, Relay, SendOptions, SpawnOptions, SystemActorError};
use bioma_mcp::{
    schema::CallToolResult,
    server::RequestContext,
    tools::{ToolDef, ToolError},
};
use bioma_rag::prelude::{DeleteSource as DeleteSourceArgs, Indexer};
use serde::Serialize;
use std::borrow::Cow;
use std::time::Duration;
use tracing::error;

#[derive(Serialize)]
pub struct DeleteSourceTool {
    #[serde(skip_serializing)]
    id: ActorId,
    #[serde(skip_serializing)]
    engine: Engine,
}

impl DeleteSourceTool {
    pub async fn new(engine: &Engine) -> Result<Self, SystemActorError> {
        let id = ActorId::of::<Indexer>("/rag/indexer");

        let (mut indexer_ctx, mut indexer_actor) =
            Actor::spawn(engine.clone(), id.clone(), Indexer::default(), SpawnOptions::default()).await.map_err(
                |e| SystemActorError::LiveStream(Cow::Owned(format!("Failed to spawn indexer actor: {}", e))),
            )?;

        tokio::spawn(async move {
            if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
                error!("Indexer actor error: {}", e);
            }
        });

        Ok(Self { id, engine: engine.clone() })
    }
}

impl ToolDef for DeleteSourceTool {
    const NAME: &'static str = "delete_source";
    const DESCRIPTION: &'static str = "Delete indexed sources and their associated embeddings";
    type Args = DeleteSourceArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        let relay_id = ActorId::of::<Relay>("/rag/indexer/relay");

        let (relay_ctx, _) = Actor::spawn(self.engine.clone(), relay_id, Relay, SpawnOptions::default())
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to spawn relay: {}", e)))?;

        let delete_source = DeleteSourceArgs { sources: args.sources, delete_from_disk: args.delete_from_disk };

        let response = relay_ctx
            .send_and_wait_reply::<Indexer, DeleteSourceArgs>(
                delete_source,
                &self.id,
                SendOptions::builder().timeout(Duration::from_secs(200)).build(),
            )
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to delete sources: {}", e)))?;

        let response_value = serde_json::to_value(response)
            .map_err(|e| ToolError::Execution(format!("Failed to serialize response: {}", e)))?;

        Ok(CallToolResult { meta: None, content: vec![response_value], is_error: None })
    }
}
