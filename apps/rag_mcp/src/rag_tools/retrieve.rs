use bioma_actor::{Actor, ActorId, Engine, Relay, SendOptions, SpawnOptions, SystemActorError};
use bioma_mcp::{
    schema::CallToolResult,
    server::RequestContext,
    tools::{ToolDef, ToolError},
};
use bioma_rag::prelude::{RetrieveContext as RetrieveContextArgs, Retriever};
use serde::Serialize;
use std::borrow::Cow;
use std::time::Duration;
use tracing::error;

#[derive(Serialize)]
pub struct RetrieveTool {
    #[serde(skip_serializing)]
    id: ActorId,
    #[serde(skip_serializing)]
    engine: Engine,
}

impl RetrieveTool {
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

impl ToolDef for RetrieveTool {
    const NAME: &'static str = "retrieve";
    const DESCRIPTION: &'static str = "Retrieves context from indexed sources based on semantic similarity";
    type Args = RetrieveContextArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        let relay_id = ActorId::of::<Relay>("/rag/retriever/relay");

        let (relay_ctx, _) = Actor::spawn(self.engine.clone(), relay_id, Relay, SpawnOptions::default())
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to spawn relay: {}", e)))?;

        let response = relay_ctx
            .send_and_wait_reply::<Retriever, RetrieveContextArgs>(
                args,
                &self.id,
                SendOptions::builder().timeout(Duration::from_secs(200)).build(),
            )
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to retrieve context: {}", e)))?;

        // Convert the response to the appropriate output format based on request
        let response_value = match response.to_json() {
            json => serde_json::from_str(&json)
                .map_err(|e| ToolError::Execution(format!("Failed to parse response: {}", e)))?,
        };

        Ok(CallToolResult { meta: None, content: vec![response_value], is_error: None })
    }
}
