use bioma_actor::prelude::*;
use ollama_rs::{error::OllamaError, generation::options::GenerationOptions, Ollama};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

#[derive(thiserror::Error, Debug)]
pub enum EmbeddingsError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Ollama error: {0}")]
    Ollama(#[from] OllamaError),
    #[error("Ollama not initialized")]
    OllamaNotInitialized,
}

impl ActorError for EmbeddingsError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateEmbeddings {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedEmbeddings {
    pub embeddings: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Embeddings {
    pub model_name: String,
    pub generation_options: Option<GenerationOptions>,
    #[serde(skip)]
    pub ollama: Option<Ollama>,
}

impl Message<GenerateEmbeddings> for Embeddings {
    type Response = GeneratedEmbeddings;

    fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        message: &GenerateEmbeddings,
    ) -> impl Future<Output = Result<GeneratedEmbeddings, EmbeddingsError>> {
        async move {
            let Some(ollama) = &self.ollama else {
                return Err(EmbeddingsError::OllamaNotInitialized);
            };

            let result = ollama
                .generate_embeddings(self.model_name.clone(), message.text.clone(), self.generation_options.clone())
                .await?;
            let embeddings = result.embeddings.into_iter().map(|e| e as f32).collect::<Vec<f32>>();

            Ok(GeneratedEmbeddings { embeddings })
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SystemMessage {
    Exit,
}

impl Message<SystemMessage> for Embeddings {
    type Response = ();

    fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        _message: &SystemMessage,
    ) -> impl Future<Output = Result<(), EmbeddingsError>> {
        async move { Ok(()) }
    }
}

impl Actor for Embeddings {
    type Error = EmbeddingsError;

    fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), EmbeddingsError>> {
        async move {
            info!("{} Started", ctx.id());

            self.ollama = Some(Ollama::default());

            let mut stream = ctx.recv().await?;
            while let Some(Ok(frame)) = stream.next().await {
                if let Some(input) = frame.is::<GenerateEmbeddings>() {
                    let response = self.reply(ctx, &input, &frame).await;
                    if let Err(err) = response {
                        error!("{} {:?}", ctx.id(), err);
                    }
                } else if let Some(sys_msg) = frame.is::<SystemMessage>() {
                    self.reply(ctx, &sys_msg, &frame).await?;
                    match sys_msg {
                        SystemMessage::Exit => break,
                    }
                }
            }
            info!("{} Finished", ctx.id());
            Ok(())
        }
    }
}
