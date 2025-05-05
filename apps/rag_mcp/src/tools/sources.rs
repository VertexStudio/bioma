use bioma_actor::{Actor, ActorId, Engine, Relay, SendOptions, SpawnOptions, SystemActorError};
use bioma_mcp::{
    schema::CallToolResult,
    server::RequestContext,
    tools::{ToolDef, ToolError},
};
use bioma_rag::prelude::{ListSources, Retriever};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::time::Duration;
use tracing::error;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ListSourcesArgs {}

#[derive(Serialize)]
pub struct SourcesTool {
    #[serde(skip_serializing)]
    id: ActorId,
    #[serde(skip_serializing)]
    engine: Engine,
}

impl SourcesTool {
    pub async fn new(engine: &Engine) -> Result<Self, SystemActorError> {
        let id = ActorId::of::<Retriever>("/rag/retriever");

        let (mut retriever_ctx, mut retriever_actor) =
            Actor::spawn(engine.clone(), id.clone(), Retriever::default(), SpawnOptions::default()).await.map_err(
                |e| SystemActorError::LiveStream(Cow::Owned(format!("Failed to spawn retriever actor: {}", e))),
            )?;

        tokio::spawn(async move {
            if let Err(e) = retriever_actor.start(&mut retriever_ctx).await {
                error!("Retriever actor error: {}", e);
            }
        });

        Ok(Self { id, engine: engine.clone() })
    }
}

impl ToolDef for SourcesTool {
    const NAME: &'static str = "list_sources";
    const DESCRIPTION: &'static str = "List indexed sources available for retrieval";
    type Args = ListSourcesArgs;

    async fn call(&self, _args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        let relay_id = ActorId::of::<Relay>("/rag/retriever/relay");

        let (relay_ctx, _) = Actor::spawn(self.engine.clone(), relay_id, Relay, SpawnOptions::default())
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to spawn relay: {}", e)))?;

        let response = relay_ctx
            .send_and_wait_reply::<Retriever, ListSources>(
                ListSources,
                &self.id,
                SendOptions::builder().timeout(Duration::from_secs(60)).build(),
            )
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to list sources: {}", e)))?;

        let response_value = serde_json::to_value(response)
            .map_err(|e| ToolError::Execution(format!("Failed to serialize response: {}", e)))?;

        Ok(CallToolResult { meta: None, content: vec![response_value], is_error: None })
    }
}
