use bioma_actor::{Actor, ActorId, Engine, Relay, SendOptions, SpawnOptions, SystemActorError};
use bioma_mcp::schema::CallToolResult;
use bioma_mcp::server::RequestContext;
use bioma_mcp::tools::{ToolDef, ToolError};
use bioma_rag::prelude::{EmbeddingContent, Embeddings, GenerateEmbeddings, ImageData};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::time::Duration;
use tracing::error;

/// Request schema for generating embeddings
#[derive(schemars::JsonSchema, Serialize, Deserialize)]
pub struct EmbeddingsQueryArgs {
    /// The embedding model to use
    pub model: ModelEmbed,

    /// The input data to generate embeddings for (text or base64-encoded image)
    pub input: serde_json::Value,
}

#[derive(schemars::JsonSchema, Serialize, Deserialize)]
pub enum ModelEmbed {
    #[serde(rename = "nomic-embed-text")]
    NomicEmbedTextV15,
    #[serde(rename = "nomic-embed-vision")]
    NomicEmbedVisionV15,
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
    const DESCRIPTION: &'static str = "Generate embeddings for text or images";
    type Args = EmbeddingsQueryArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        let relay_id = ActorId::of::<Relay>("/rag/embeddings/relay");

        let (relay_ctx, _) = Actor::spawn(self.engine.clone(), relay_id, Relay, SpawnOptions::default())
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to spawn relay: {}", e)))?;

        let embedding_content = match args.model {
            ModelEmbed::NomicEmbedTextV15 => {
                let texts = match args.input.as_str() {
                    Some(s) => vec![s.to_string()],
                    None => match args.input.as_array() {
                        Some(arr) => arr.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
                        None => {
                            return Err(ToolError::Execution("Input must be string or array of strings".to_string()));
                        }
                    },
                };
                EmbeddingContent::Text(texts)
            }
            ModelEmbed::NomicEmbedVisionV15 => {
                let base64_images = match args.input.as_str() {
                    Some(s) => vec![s.to_string()],
                    None => match args.input.as_array() {
                        Some(arr) => arr.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
                        None => {
                            return Err(ToolError::Execution("Input must be string or array of strings".to_string()));
                        }
                    },
                };

                let images = base64_images.into_iter().map(ImageData::Base64).collect();
                EmbeddingContent::Image(images)
            }
        };

        let response = relay_ctx
            .send_and_wait_reply::<Embeddings, GenerateEmbeddings>(
                GenerateEmbeddings { content: embedding_content },
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
