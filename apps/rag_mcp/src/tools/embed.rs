use anyhow::{Error, anyhow};
use bioma_actor::{Actor, ActorId, Engine, Relay, SendOptions, SpawnExistsOptions, SpawnOptions, SystemActorError};
use bioma_mcp::schema::CallToolResult;
use bioma_mcp::server::RequestContext;
use bioma_mcp::tools::ToolDef;
use bioma_rag::prelude::{EmbeddingContent, Embeddings, GenerateEmbeddings, ImageData};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::time::Duration;
use tracing::error;

#[derive(schemars::JsonSchema, Serialize, Deserialize)]
pub struct EmbeddingsQueryArgs {
    pub model: ModelEmbed,
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

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, Error> {
        let relay_id = ActorId::of::<Relay>("/rag/embeddings/relay");

        let (relay_ctx, _) = Actor::spawn(
            self.engine.clone(),
            relay_id,
            Relay,
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;

        let embedding_content = match args.model {
            ModelEmbed::NomicEmbedTextV15 => {
                let texts = match args.input.as_str() {
                    Some(s) => vec![s.to_string()],
                    None => match args.input.as_array() {
                        Some(arr) => arr.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
                        None => {
                            return Err(anyhow!("Input must be string or array of strings"));
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
                            return Err(anyhow!("Input must be string or array of strings"));
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
            .await?;

        let response_value = serde_json::to_value(response)?;

        Ok(CallToolResult { meta: None, content: vec![response_value], is_error: None })
    }
}

#[cfg(test)]
mod tests {
    use bioma_actor::Engine;
    use bioma_mcp::server::RequestContext;
    use bioma_mcp::tools::ToolDef;
    use bioma_rag::prelude::GeneratedEmbeddings;
    use serde_json::json;

    use crate::tools::embed::{EmbedTool, EmbeddingsQueryArgs, ModelEmbed};

    #[tokio::test]
    async fn generate_text_embeddings() {
        let engine = bioma_actor::Engine::test().await.unwrap();
        let embed_tool = EmbedTool::new(&engine).await.unwrap();

        let args = EmbeddingsQueryArgs { model: ModelEmbed::NomicEmbedTextV15, input: json!(["Hello from Sergio!"]) };

        let raw =
            embed_tool.call(args, RequestContext::default()).await.unwrap_or_else(|e| panic!("tool failed: {e:?}"));

        let out: GeneratedEmbeddings = serde_json::from_value(raw.content[0].clone()).unwrap();

        assert_eq!(out.embeddings.len(), 1, "exactly one embedding returned for one input string");
        assert!(!out.embeddings[0].is_empty(), "each embedding vector should be non-empty");
    }

    #[tokio::test]
    async fn generate_image_embeddings() {
        const IMAGE: &str =
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

        let engine = Engine::test().await.unwrap();
        let embed_tool = EmbedTool::new(&engine).await.unwrap();

        let args = EmbeddingsQueryArgs { model: ModelEmbed::NomicEmbedVisionV15, input: json!(IMAGE) };

        let raw =
            embed_tool.call(args, RequestContext::default()).await.unwrap_or_else(|e| panic!("tool failed: {e:?}"));

        let out: GeneratedEmbeddings = serde_json::from_value(raw.content[0].clone()).unwrap();

        assert_eq!(out.embeddings.len(), 1, "one image ⇒ one embedding");
        assert!(!out.embeddings[0].is_empty(), "vision embedding vector should be non-empty");
    }
}
