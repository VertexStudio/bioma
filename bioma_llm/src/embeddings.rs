use bioma_actor::prelude::*;
use derive_more::Display;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use surrealdb::value::RecordId;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{error, info};

const DEFAULT_EMBEDDING_LENGTH: usize = 768;

#[derive(thiserror::Error, Debug)]
pub enum EmbeddingsError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Url error: {0}")]
    Url(#[from] url::ParseError),
    #[error("No embeddings generated")]
    NoEmbeddingsGenerated,
    #[error("Model name not set")]
    ModelNameNotSet,
    #[error("Fastembed error: {0}")]
    Fastembed(#[from] fastembed::Error),
    #[error("Text embedding not initialized")]
    TextEmbeddingNotInitialized,
    #[error("Error sending text embeddings: {0}")]
    SendTextEmbeddings(#[from] mpsc::error::SendError<TextEmbeddingRequest>),
    #[error("Error receiving text embeddings: {0}")]
    RecvTextEmbeddings(#[from] oneshot::error::RecvError),
}

pub struct TextEmbeddingRequest {
    sender: oneshot::Sender<Result<Vec<Vec<f32>>, fastembed::Error>>,
    texts: Vec<String>,
}

impl ActorError for EmbeddingsError {}

/// Generate embeddings for a set of texts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateTextEmbeddings {
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
pub struct GeneratedTextEmbeddings {
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

#[derive(bon::Builder, Debug, Serialize, Deserialize)]
pub struct Embeddings {
    #[builder(default = Model::NomicEmbedTextV15)]
    pub model: Model,
    #[serde(skip)]
    text_embedding_task: Option<(mpsc::Sender<TextEmbeddingRequest>, JoinHandle<Result<(), fastembed::Error>>)>,
}

impl Clone for Embeddings {
    fn clone(&self) -> Self {
        Self { model: self.model.clone(), text_embedding_task: None }
    }
}

impl Default for Embeddings {
    fn default() -> Self {
        Self { model: Model::NomicEmbedTextV15, text_embedding_task: None }
    }
}

impl Message<TopK> for Embeddings {
    type Response = Vec<Similarity>;

    async fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        message: &TopK,
    ) -> Result<Vec<Similarity>, EmbeddingsError> {
        let Some((text_embedding_tx, _)) = self.text_embedding_task.as_ref() else {
            return Err(EmbeddingsError::TextEmbeddingNotInitialized);
        };

        // Generate embedding for query if not already an embedding
        let query_embedding = match &message.query {
            Query::Embedding(embedding) => embedding.clone(),
            Query::Text(text) => {
                let (tx, rx) = oneshot::channel();
                text_embedding_tx.send(TextEmbeddingRequest { sender: tx, texts: vec![text.to_string()] }).await?;
                rx.await??.first().cloned().ok_or(EmbeddingsError::NoEmbeddingsGenerated)?
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

impl Message<GenerateTextEmbeddings> for Embeddings {
    type Response = GeneratedTextEmbeddings;

    async fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        message: &GenerateTextEmbeddings,
    ) -> Result<GeneratedTextEmbeddings, EmbeddingsError> {
        let Some((text_embedding_tx, _)) = self.text_embedding_task.as_ref() else {
            return Err(EmbeddingsError::TextEmbeddingNotInitialized);
        };

        let (tx, rx) = oneshot::channel();
        text_embedding_tx.send(TextEmbeddingRequest { sender: tx, texts: message.texts.clone() }).await?;
        let embeddings = rx.await??;

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
                let embedding = embeddings[i].clone();
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

        Ok(GeneratedTextEmbeddings { embeddings })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Display)]
pub enum Model {
    NomicEmbedTextV15,
}

fn get_fastembed_model(model: &Model) -> fastembed::EmbeddingModel {
    match model {
        Model::NomicEmbedTextV15 => fastembed::EmbeddingModel::NomicEmbedTextV15,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: Model,
    pub dim: usize,
    pub description: String,
    pub model_code: String,
    pub model_file: String,
}

impl Actor for Embeddings {
    type Error = EmbeddingsError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), EmbeddingsError> {
        info!("{} Started", ctx.id());

        // Spawn the text embedding task
        let model = self.model.clone();
        let (tx, mut rx) = mpsc::channel::<TextEmbeddingRequest>(100);
        let text_embedding_task: JoinHandle<Result<(), fastembed::Error>> = tokio::task::spawn_blocking(move || {
            let text_embedding = fastembed::TextEmbedding::try_new(
                fastembed::InitOptions::new(get_fastembed_model(&model)).with_show_download_progress(true),
            )?;
            while let Some(request) = rx.blocking_recv() {
                let embeddings = text_embedding.embed(request.texts, None);
                let _ = request.sender.send(embeddings);
            }
            Ok(())
        });

        self.text_embedding_task = Some((tx, text_embedding_task));

        let fastembed_model = get_fastembed_model(&self.model);
        let model_info = fastembed::TextEmbedding::get_model_info(&fastembed_model)?;
        let model_info = ModelInfo {
            name: self.model.clone(),
            dim: model_info.dim,
            description: model_info.description.clone(),
            model_code: model_info.model_code.clone(),
            model_file: model_info.model_file.clone(),
        };

        // Assert embedding length
        if model_info.dim != DEFAULT_EMBEDDING_LENGTH {
            panic!("Embedding length must be 768");
        }

        // Define the schema
        let def = include_str!("../sql/def.surql");
        let db = ctx.engine().db();
        db.query(def).await.map_err(SystemActorError::from)?;

        // Store model in database if not already present
        let model: Result<Option<Record>, _> =
            db.create(("model", self.model.to_string())).content(model_info).await.map_err(SystemActorError::from);
        if let Ok(Some(model)) = &model {
            info!("Model {:?} stored with id: {}", self.model, model.id);
        }

        // Start the message stream
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(input) = frame.is::<GenerateTextEmbeddings>() {
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
