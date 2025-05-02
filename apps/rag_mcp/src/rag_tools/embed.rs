use bioma_actor::{Actor, ActorId, Engine, Relay, SendOptions, SpawnOptions, SystemActorError};
use bioma_mcp::{
    schema::CallToolResult,
    server::RequestContext,
    tools::{ToolDef, ToolError},
};
use bioma_rag::prelude::{EmbeddingContent, Embeddings, GenerateEmbeddings};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::time::Duration;
use tracing::error;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct EmbedArgs {
    #[schemars(description = "Text to embed")]
    text: String,

    #[schemars(description = "Model to use for embeddings")]
    model: Option<String>,

    #[schemars(description = "Whether to normalize the embeddings")]
    normalize: Option<bool>,
}

#[derive(Serialize)]
pub struct EmbedTool {
    #[serde(skip_serializing)]
    id: ActorId,
    #[serde(skip_serializing)]
    engine: Engine,
}

impl EmbedTool {
    pub async fn new(engine: &Engine) -> Result<Self, SystemActorError> {
        let id = ActorId::of::<Embeddings>("/rag/embeddings");

        let (mut embeddings_ctx, mut embeddings_actor) =
            Actor::spawn(engine.clone(), id.clone(), Embeddings::default(), SpawnOptions::default()).await.map_err(
                |e| SystemActorError::LiveStream(Cow::Owned(format!("Failed to spawn embeddings actor: {}", e))),
            )?;

        tokio::spawn(async move {
            if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
                error!("Embeddings actor error: {}", e);
            }
        });

        Ok(Self { id, engine: engine.clone() })
    }
}

impl ToolDef for EmbedTool {
    const NAME: &'static str = "embed";
    const DESCRIPTION: &'static str = "Generate embeddings for text";
    type Args = EmbedArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        let relay_id = ActorId::of::<Relay>("/rag/embeddings/relay");

        let (relay_ctx, _) = Actor::spawn(self.engine.clone(), relay_id, Relay, SpawnOptions::default())
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to spawn relay: {}", e)))?;

        // Create embedding request
        let generate_embeddings = GenerateEmbeddings { content: EmbeddingContent::Text(vec![args.text]) };

        let response = relay_ctx
            .send_and_wait_reply::<Embeddings, GenerateEmbeddings>(
                generate_embeddings,
                &self.id,
                SendOptions::builder().timeout(Duration::from_secs(200)).build(),
            )
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to generate embeddings: {}", e)))?;

        let response_value = serde_json::to_value(response)
            .map_err(|e| ToolError::Execution(format!("Failed to serialize response: {}", e)))?;

        Ok(CallToolResult { meta: None, content: vec![response_value], is_error: None })
    }
}
