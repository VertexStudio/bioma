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
use serde_json::Value;
use std::borrow::Cow;
use surrealdb::value::RecordId;
use tracing::{error, info};
use url::Url;

const DEFAULT_MODEL_NAME: &str = "nomic-embed-text";
const DEFAULT_ENDPOINT: &str = "http://localhost:11434";
const DEFAULT_EMBEDDING_LENGTH: usize = 768;

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
    #[error("No embeddings generated")]
    NoEmbeddingsGenerated,
    #[error("Model name not set")]
    ModelNameNotSet,
}

impl ActorError for EmbeddingsError {}

/// Generate embeddings for a set of texts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateEmbeddings {
    /// Source of the embeddings
    pub source: String,
    /// The texts to embed
    pub texts: Vec<String>,
    /// Metadata to store with the embeddings
    pub metadata: Option<Vec<Value>>,
    /// The tag to store the embeddings with
    pub tag: Option<String>,
}

/// The generated embeddings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedEmbeddings {
    pub embeddings: Vec<Vec<f32>>,
}

/// The query to search for
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Query {
    Embedding(Vec<f32>),
    Text(String),
}

/// Get the top k similar embeddings to a query, filtered by tag
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopK {
    /// The query to search for
    pub query: Query,
    /// The tag to filter the embeddings by
    pub tag: Option<String>,
    /// Number of similar embeddings to return
    pub k: usize,
    /// The threshold for the similarity score
    pub threshold: f32,
}

/// The similarity between a query and an embedding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Similarity {
    pub text: String,
    pub similarity: f32,
    pub metadata: Option<Value>,
}

#[derive(bon::Builder, Debug, Clone, Serialize, Deserialize)]
pub struct Embeddings {
    #[builder(default = DEFAULT_MODEL_NAME.into())]
    pub model: Cow<'static, str>,
    pub generation_options: Option<GenerationOptions>,
    #[builder(default = Url::parse(DEFAULT_ENDPOINT).unwrap())]
    pub endpoint: Url,
    #[serde(skip)]
    #[builder(default)]
    ollama: Ollama,
    #[builder(default = DEFAULT_EMBEDDING_LENGTH)]
    embedding_length: usize,
}

impl Default for Embeddings {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL_NAME.into(),
            generation_options: None,
            endpoint: Url::parse(DEFAULT_ENDPOINT).unwrap(),
            ollama: Ollama::default(),
            embedding_length: DEFAULT_EMBEDDING_LENGTH,
        }
    }
}

impl Message<TopK> for Embeddings {
    type Response = Vec<Similarity>;

    async fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        message: &TopK,
    ) -> Result<Vec<Similarity>, EmbeddingsError> {
        // Generate embedding for query if not already an embedding
        let query_embedding = match &message.query {
            Query::Embedding(embedding) => embedding.clone(),
            Query::Text(text) => {
                let input = EmbeddingsInput::Single(text.to_string());
                let request = GenerateEmbeddingsRequest::new(self.model.to_string(), input);
                let result = self.ollama.generate_embeddings(request).await?;
                if result.embeddings.is_empty() {
                    return Err(EmbeddingsError::NoEmbeddingsGenerated);
                }
                result.embeddings[0].clone()
            }
        };

        // Get the similar embeddings
        let query_sql = include_str!("../sql/similarities.surql");
        let db = _ctx.engine().db();
        let mut results = db
            .query(query_sql)
            .bind(("query", query_embedding))
            .bind(("top_k", message.k.clone()))
            .bind(("tag", message.tag.clone()))
            .bind(("threshold", message.threshold))
            .await
            .map_err(SystemActorError::from)?;
        let results: Result<Vec<Similarity>, _> = results.take(0).map_err(SystemActorError::from);
        results.map_err(EmbeddingsError::from)
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

        let request = GenerateEmbeddingsRequest::new(self.model.to_string(), input);

        let result = self.ollama.generate_embeddings(request).await?;

        if let Some(tag) = &message.tag {
            let db = _ctx.engine().db();
            let emb_query = include_str!("../sql/embeddings.surql");

            // Check if metadata is same length as texts
            if let Some(metadata) = &message.metadata {
                if metadata.len() != message.texts.len() {
                    error!("Metadata length does not match texts length");
                }
            }

            // Store embeddings
            for (i, text) in message.texts.iter().enumerate() {
                let metadata = message.metadata.as_ref().map(|m| m[i].clone()).unwrap_or(Value::Null);
                let embedding = result.embeddings[i].clone();
                let model_id = RecordId::from_table_key("model", self.model.to_string());
                db.query(emb_query)
                    .bind(("tag", tag.to_string()))
                    .bind(("text", text.to_string()))
                    .bind(("embedding", embedding))
                    .bind(("metadata", metadata))
                    .bind(("model_id", model_id))
                    .bind(("source", message.source.clone()))
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

        // Connect to ollama
        self.ollama = Ollama::from_url(self.endpoint.clone());

        if self.embedding_length != DEFAULT_EMBEDDING_LENGTH {
            panic!("Embedding length must be 768");
        }

        // Define the schema
        let def = include_str!("../sql/def.surql");
        let db = ctx.engine().db();
        db.query(def).await.map_err(SystemActorError::from)?;

        // Store model in database if not already present
        let model_name = self.model.to_string();
        let model_info: ollama_rs::models::ModelInfo = self.ollama.show_model_info(model_name.clone()).await?;
        let model = Model { name: model_name.clone(), info: model_info };
        let model: Result<Option<Record>, _> =
            db.create(("model", model_name.clone())).content(model).await.map_err(SystemActorError::from);
        if let Ok(Some(model)) = model {
            info!("Model {} stored with id: {}", model_name, model.id);
        }

        // Start the message stream
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(input) = frame.is::<GenerateEmbeddings>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            } else if let Some(input) = frame.is::<TopK>() {
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
