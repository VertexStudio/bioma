use bioma_actor::prelude::*;
use ollama_rs::{
    error::OllamaError,
    generation::{
        embeddings::request::{EmbeddingsInput, GenerateEmbeddingsRequest},
        options::GenerationOptions,
    },
    Ollama,
};
use serde::{Deserialize, Serialize};
use surrealdb::value::RecordId;
use tracing::{error, info};

#[derive(thiserror::Error, Debug)]
pub enum EmbeddingsError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Ollama error: {0}")]
    Ollama(#[from] OllamaError),
    #[error("Ollama not initialized")]
    OllamaNotInitialized,
    #[error("Url error: {0}")]
    Url(#[from] url::ParseError),
}

impl ActorError for EmbeddingsError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateEmbeddings {
    pub texts: Vec<String>,
    pub store: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedEmbeddings {
    pub embeddings: Vec<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Embeddings {
    pub model_name: String,
    pub generation_options: Option<GenerationOptions>,
    #[serde(skip)]
    pub ollama: Ollama,
    #[serde(skip)]
    pub embedding_length: usize,
}

impl Default for Embeddings {
    fn default() -> Self {
        Self {
            model_name: "nomic-embed-text".to_string(),
            generation_options: None,
            ollama: Ollama::default(),
            embedding_length: 768,
        }
    }
}

impl Message<GenerateEmbeddings> for Embeddings {
    type Response = GeneratedEmbeddings;

    async fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        message: &GenerateEmbeddings,
    ) -> Result<GeneratedEmbeddings, EmbeddingsError> {
        let input = EmbeddingsInput::Multiple(message.texts.clone());

        let request = GenerateEmbeddingsRequest::new(self.model_name.clone(), input);

        let result = self.ollama.generate_embeddings(request).await?;

        if message.store {
            let db = _ctx.engine().db();
            let emb_query = include_str!("../sql/embeddings.surql");
            for (i, text) in message.texts.iter().enumerate() {
                let embedding = result.embeddings[i].clone();
                let model_id = RecordId::from_table_key("model", self.model_name.clone());
                db.query(emb_query)
                    .bind(("text", text.to_string()))
                    .bind(("embedding", embedding))
                    .bind(("model_id", model_id))
                    .await
                    .map_err(SystemActorError::from)?;
            }
        }

        Ok(GeneratedEmbeddings { embeddings: result.embeddings })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub name: String,
    pub info: ollama_rs::models::ModelInfo,
}

impl Actor for Embeddings {
    type Error = EmbeddingsError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), EmbeddingsError> {
        info!("{} Started", ctx.id());

        if self.embedding_length != 768 {
            panic!("Embedding length must be 768");
        }

        // Define the schema
        let def = include_str!("../sql/def.surql");
        let db = ctx.engine().db();
        db.query(def).await.map_err(SystemActorError::from)?;

        // Store model in database if not already present
        let model_info: ollama_rs::models::ModelInfo = self.ollama.show_model_info(self.model_name.clone()).await?;
        let model = Model { name: self.model_name.clone(), info: model_info };
        let model: Option<Record> =
            db.create(("model", self.model_name.clone())).content(model).await.map_err(SystemActorError::from)?;
        if let Some(model) = model {
            info!("Model {} stored with id: {}", self.model_name, model.id);
        }

        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(input) = frame.is::<GenerateEmbeddings>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            }
        }
        info!("{} Finished", ctx.id());
        Ok(())
    }
}
