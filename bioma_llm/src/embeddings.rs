// use crate::ORT_EXIT_MUTEX;
use bioma_actor::prelude::*;
use derive_more::{Deref, Display};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Weak};
use surrealdb::value::RecordId;
use tokio::sync::Mutex;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{error, info};

pub const DEFAULT_TEXT_EMBEDDING_LENGTH: usize = 768;
pub const DEFAULT_IMAGE_EMBEDDING_LENGTH: usize = 512;
lazy_static! {
    static ref SHARED_EMBEDDING: Arc<Mutex<Weak<SharedEmbedding>>> = Arc::new(Mutex::new(Weak::new()));
}

struct SharedEmbedding {
    text_embedding_tx: mpsc::Sender<TextEmbeddingRequest>,
    image_embedding_tx: mpsc::Sender<ImageEmbeddingRequest>,
}

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
    RecvEmbeddings(#[from] oneshot::error::RecvError),
    #[error("Error sending image embeddings: {0}")]
    SendImageEmbeddings(#[from] mpsc::error::SendError<ImageEmbeddingRequest>),
    #[error("Image embedding not initialized")]
    ImageEmbeddingNotInitialized,
}

pub struct TextEmbeddingRequest {
    response_tx: oneshot::Sender<Result<Vec<Vec<f32>>, fastembed::Error>>,
    texts: Vec<String>,
}

pub struct ImageEmbeddingRequest {
    response_tx: oneshot::Sender<Result<Vec<Vec<f32>>, fastembed::Error>>,
    images: Vec<Image>,
}

impl ActorError for EmbeddingsError {}

/// Generate embeddings for a set of texts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreTextEmbeddings {
    /// Source of the embeddings
    pub source: String,
    /// The texts to embed
    pub texts: Vec<String>,
    /// Metadata to store with the embeddings
    pub metadata: Option<Vec<Value>>,
    /// The tag to store the embeddings with
    pub tag: Option<String>,
}

/// Store embeddings for a set of images
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreImageEmbeddings {
    /// Source of the embeddings
    pub source: String,
    /// The images to embed
    pub images: Vec<Image>,
    /// Metadata to store with the embeddings
    pub metadata: Option<Vec<Value>>,
    /// The tag to store the embeddings with
    pub tag: Option<String>,
}

/// Generate embeddings for a set of images
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateImageEmbeddings {
    /// The images to embed
    pub images: Vec<Image>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Image {
    pub path: String,
    pub caption: Option<String>,
}

impl AsRef<Path> for Image {
    fn as_ref(&self) -> &Path {
        Path::new(&self.path)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredEmbeddings {
    pub lengths: Vec<usize>,
}

/// Generate embeddings for a set of texts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateTextEmbeddings {
    /// The texts to embed
    pub texts: Vec<String>,
}

/// The generated embeddings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedTextEmbeddings {
    pub embeddings: Vec<Vec<f32>>,
}

/// The query to search for
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Query {
    TextEmbedding(Vec<f32>),
    ImageEmbedding(Vec<f32>),
    Text(String),
    Image(String),
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
    pub text_model: Model,
    #[builder(default = ImageModel::ClipVitB32)]
    pub image_model: ImageModel,
    #[serde(skip)]
    text_embedding_tx: Option<mpsc::Sender<TextEmbeddingRequest>>,
    #[serde(skip)]
    image_embedding_tx: Option<mpsc::Sender<ImageEmbeddingRequest>>,
    #[serde(skip)]
    shared_embedding: Option<StrongSharedEmbedding>,
}

#[derive(Deref)]
struct StrongSharedEmbedding(Arc<SharedEmbedding>);

impl std::fmt::Debug for StrongSharedEmbedding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StrongSharedEmbedding")
    }
}

impl Clone for Embeddings {
    fn clone(&self) -> Self {
        Self {
            text_model: self.text_model.clone(),
            image_model: self.image_model.clone(),
            text_embedding_tx: None,
            image_embedding_tx: None,
            shared_embedding: None,
        }
    }
}

impl Default for Embeddings {
    fn default() -> Self {
        Embeddings::builder().build()
    }
}

impl Message<TopK> for Embeddings {
    type Response = Vec<Similarity>;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: &TopK,
    ) -> Result<Vec<Similarity>, EmbeddingsError> {
        let query_embedding = match &message.query {
            Query::TextEmbedding(embedding) | Query::ImageEmbedding(embedding) => embedding.clone(),
            Query::Text(text) | Query::Image(text) => {
                let Some(text_embedding_tx) = self.text_embedding_tx.as_ref() else {
                    return Err(EmbeddingsError::TextEmbeddingNotInitialized);
                };
                let (tx, rx) = oneshot::channel();
                text_embedding_tx.send(TextEmbeddingRequest { response_tx: tx, texts: vec![text.to_string()] }).await?;
                rx.await??.first().cloned().ok_or(EmbeddingsError::NoEmbeddingsGenerated)?
            }
        };

        // TODO: Queries are similar. See if we can use just one.
        // Get the similar embeddings using the appropriate query
        let query_sql = match message.query {
            Query::TextEmbedding(_) | Query::Text(_) => {
                include_str!("../sql/similarities/similarities_text.surql")
            }
            _ => {
                include_str!("../sql/similarities/similarities_image.surql")
            }
        };

        let db = ctx.engine().db();
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

impl Message<StoreTextEmbeddings> for Embeddings {
    type Response = StoredEmbeddings;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: &StoreTextEmbeddings,
    ) -> Result<StoredEmbeddings, EmbeddingsError> {
        let Some(text_embedding_tx) = self.text_embedding_tx.as_ref() else {
            return Err(EmbeddingsError::TextEmbeddingNotInitialized);
        };

        let (tx, rx) = oneshot::channel();
        text_embedding_tx.send(TextEmbeddingRequest { response_tx: tx, texts: message.texts.clone() }).await?;
        let embeddings = rx.await??;

        let db = ctx.engine().db();
        let emb_query = include_str!("../sql/embeddings/store_text_embedding.surql");

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
            let model_id = RecordId::from_table_key("model", self.text_model.to_string());
            db.query(emb_query)
                .bind(("tag", message.tag.clone()))
                .bind(("text", text.to_string()))
                .bind(("embedding", embedding))
                .bind(("metadata", metadata))
                .bind(("model_id", model_id))
                .bind(("source", message.source.clone()))
                .await
                .map_err(SystemActorError::from)?;
        }

        Ok(StoredEmbeddings { lengths: embeddings.iter().map(|e| e.len()).collect() })
    }
}

impl Message<GenerateTextEmbeddings> for Embeddings {
    type Response = GeneratedTextEmbeddings;

    async fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        message: &GenerateTextEmbeddings,
    ) -> Result<GeneratedTextEmbeddings, EmbeddingsError> {
        let Some(text_embedding_tx) = self.text_embedding_tx.as_ref() else {
            return Err(EmbeddingsError::TextEmbeddingNotInitialized);
        };

        let (tx, rx) = oneshot::channel();
        text_embedding_tx.send(TextEmbeddingRequest { response_tx: tx, texts: message.texts.clone() }).await?;
        let embeddings = rx.await??;

        Ok(GeneratedTextEmbeddings { embeddings })
    }
}

impl Message<GenerateImageEmbeddings> for Embeddings {
    type Response = StoredEmbeddings;

    async fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        message: &GenerateImageEmbeddings,
    ) -> Result<StoredEmbeddings, EmbeddingsError> {
        let Some(image_embedding_tx) = self.image_embedding_tx.as_ref() else {
            return Err(EmbeddingsError::TextEmbeddingNotInitialized);
        };

        let (tx, rx) = oneshot::channel();
        image_embedding_tx.send(ImageEmbeddingRequest { response_tx: tx, images: message.images.clone() }).await?;
        let embeddings = rx.await??;

        // Since this is just generate (not store), we just return the lengths
        Ok(StoredEmbeddings { lengths: embeddings.iter().map(|e| e.len()).collect() })
    }
}

impl Message<StoreImageEmbeddings> for Embeddings {
    type Response = StoredEmbeddings;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: &StoreImageEmbeddings,
    ) -> Result<StoredEmbeddings, EmbeddingsError> {
        let Some(image_embedding_tx) = self.image_embedding_tx.as_ref() else {
            return Err(EmbeddingsError::TextEmbeddingNotInitialized);
        };

        let (tx, rx) = oneshot::channel();
        image_embedding_tx.send(ImageEmbeddingRequest { response_tx: tx, images: message.images.clone() }).await?;
        let embeddings = rx.await??;

        let db = ctx.engine().db();
        let emb_query = include_str!("../sql/embeddings/store_image_embedding.surql");

        // Check if metadata is same length as image paths
        if let Some(metadata) = &message.metadata {
            if metadata.len() != message.images.len() {
                error!("Metadata length does not match images length");
            }
        }

        // Store embeddings
        for (i, image) in message.images.iter().enumerate() {
            let metadata = message.metadata.as_ref().map(|m| m[i].clone()).unwrap_or(Value::Null);
            let embedding = embeddings[i].clone();
            let model_id = RecordId::from_table_key("model", self.image_model.to_string());
            db.query(emb_query)
                .bind(("tag", message.tag.clone()))
                .bind(("caption", image.caption.clone()))
                .bind(("embedding", embedding))
                .bind(("metadata", metadata))
                .bind(("model_id", model_id))
                .bind(("source", message.source.clone()))
                .await
                .map_err(SystemActorError::from)?;
        }

        Ok(StoredEmbeddings { lengths: embeddings.iter().map(|e| e.len()).collect() })
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

#[derive(Debug, Clone, Serialize, Deserialize, Display)]
pub enum ImageModel {
    ClipVitB32,
    Resnet50,
    UnicomVitB16,
    UnicomVitB32,
}

fn get_fastembed_image_model(model: &ImageModel) -> fastembed::ImageEmbeddingModel {
    match model {
        ImageModel::ClipVitB32 => fastembed::ImageEmbeddingModel::ClipVitB32,
        ImageModel::Resnet50 => fastembed::ImageEmbeddingModel::Resnet50,
        ImageModel::UnicomVitB16 => fastembed::ImageEmbeddingModel::UnicomVitB16,
        ImageModel::UnicomVitB32 => fastembed::ImageEmbeddingModel::UnicomVitB32,
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

// Add this struct for image model info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageModelInfo {
    pub name: ImageModel,
    pub dim: usize,
    pub description: String,
    pub model_code: String,
    pub model_file: String,
}

impl Actor for Embeddings {
    type Error = EmbeddingsError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), EmbeddingsError> {
        info!("{} Started", ctx.id());

        self.init(ctx).await?;

        // Start the message stream
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(input) = frame.is::<StoreTextEmbeddings>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            } else if let Some(input) = frame.is::<GenerateTextEmbeddings>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            } else if let Some(input) = frame.is::<TopK>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            } else if let Some(input) = frame.is::<GenerateImageEmbeddings>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            } else if let Some(input) = frame.is::<StoreImageEmbeddings>() {
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

impl Embeddings {
    pub async fn init(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), EmbeddingsError> {
        info!("{} Started", ctx.id());

        // Manage a shared embedding task
        let shared_embedding = {
            let mut weak_ref = SHARED_EMBEDDING.lock().await;
            if let Some(strong_ref) = weak_ref.upgrade() {
                // Return the existing shared embedding
                Some(strong_ref)
            } else {
                let ctx_id = ctx.id().clone();

                // Get text model info
                let fastembed_model = get_fastembed_model(&self.text_model);
                let text_model_info = fastembed::TextEmbedding::get_model_info(&fastembed_model)?;
                let text_model_info = ModelInfo {
                    name: self.text_model.clone(),
                    dim: text_model_info.dim,
                    description: text_model_info.description.clone(),
                    model_code: text_model_info.model_code.clone(),
                    model_file: text_model_info.model_file.clone(),
                };

                // Get image model info
                let fastembed_image_model = get_fastembed_image_model(&self.image_model);
                let image_model_info = fastembed::ImageEmbedding::get_model_info(&fastembed_image_model);
                let image_model_info = ImageModelInfo {
                    name: self.image_model.clone(),
                    dim: image_model_info.dim,
                    description: image_model_info.description.clone(),
                    model_code: image_model_info.model_code.clone(),
                    model_file: image_model_info.model_file.clone(),
                };

                // Assert embedding lengths
                if text_model_info.dim != DEFAULT_TEXT_EMBEDDING_LENGTH {
                    panic!(
                        "Text embedding length must be {}. Not {}",
                        DEFAULT_TEXT_EMBEDDING_LENGTH, text_model_info.dim
                    );
                }
                if image_model_info.dim != DEFAULT_IMAGE_EMBEDDING_LENGTH {
                    panic!(
                        "Image embedding length must be {}. Not {}",
                        DEFAULT_IMAGE_EMBEDDING_LENGTH, image_model_info.dim
                    );
                }

                // Define the schema
                let def = include_str!("../sql/def.surql");
                let db = ctx.engine().db();
                db.query(def).await.map_err(SystemActorError::from)?;

                // Store text model info in database if not already present
                let model: Result<Option<Record>, _> = db
                    .create(("model", self.text_model.to_string()))
                    .content(text_model_info)
                    .await
                    .map_err(SystemActorError::from);
                if let Ok(Some(model)) = &model {
                    info!("Model {:?} stored with id: {}", self.text_model, model.id);
                }

                // Store image model info in database if not already present
                let model: Result<Option<Record>, _> = db
                    .create(("model", self.image_model.to_string()))
                    .content(image_model_info)
                    .await
                    .map_err(SystemActorError::from);
                if let Ok(Some(model)) = &model {
                    info!("Model {:?} stored with id: {}", self.image_model, model.id);
                }

                // Create a new shared embedding
                let model = self.text_model.clone();
                let cache_dir = ctx.engine().huggingface_cache_dir().clone();
                let (text_embedding_tx, mut text_embedding_rx) = mpsc::channel::<TextEmbeddingRequest>(100);
                let _text_embedding_task: JoinHandle<Result<(), fastembed::Error>> =
                    tokio::task::spawn_blocking(move || {
                        let mut options =
                            fastembed::InitOptions::new(get_fastembed_model(&model)).with_cache_dir(cache_dir);

                        #[cfg(target_os = "macos")]
                        {
                            options =
                                options.with_execution_providers(vec![ort::CoreMLExecutionProvider::default().build()]);
                        }

                        #[cfg(target_os = "linux")]
                        {
                            options =
                                options.with_execution_providers(vec![ort::CUDAExecutionProvider::default().build()]);
                        }

                        let text_embedding = fastembed::TextEmbedding::try_new(options)?;
                        while let Some(request) = text_embedding_rx.blocking_recv() {
                            let start = std::time::Instant::now();
                            let avg_text_len = request.texts.iter().map(|text| text.len() as f32).sum::<f32>()
                                / request.texts.len() as f32;
                            let text_count = request.texts.len();
                            match text_embedding.embed(request.texts, None) {
                                Ok(embeddings) => {
                                    info!(
                                        "Generated {} embeddings (avg. {:.1} chars) in {:?}",
                                        text_count,
                                        avg_text_len,
                                        start.elapsed()
                                    );
                                    let _ = request.response_tx.send(Ok(embeddings));
                                }
                                Err(err) => {
                                    error!("Failed to generate embeddings: {}", err);
                                    let _ = request.response_tx.send(Err(err));
                                }
                            }
                        }

                        info!("{} text embedding finished", ctx_id);
                        drop(text_embedding);

                        Ok(())
                    });

                // Setup image embeddings
                let (image_embedding_tx, mut image_embedding_rx) = mpsc::channel::<ImageEmbeddingRequest>(100);
                let image_model = self.image_model.clone();
                let cache_dir = ctx.engine().huggingface_cache_dir().clone();

                let _image_embedding_task: JoinHandle<Result<(), fastembed::Error>> =
                    tokio::task::spawn_blocking(move || {
                        let options = fastembed::ImageInitOptions::new(get_fastembed_image_model(&image_model))
                            .with_cache_dir(cache_dir);

                        let image_embedding = fastembed::ImageEmbedding::try_new(options)?;
                        while let Some(request) = image_embedding_rx.blocking_recv() {
                            let start = std::time::Instant::now();
                            let path_count = request.images.len();
                            match image_embedding.embed(request.images, None) {
                                Ok(embeddings) => {
                                    info!("Generated {} image embeddings in {:?}", path_count, start.elapsed());
                                    let _ = request.response_tx.send(Ok(embeddings));
                                }
                                Err(err) => {
                                    error!("Failed to generate image embeddings: {}", err);
                                    let _ = request.response_tx.send(Err(err));
                                }
                            }
                        }
                        Ok(())
                    });

                // Store the shared embedding
                let shared_embedding = Arc::new(SharedEmbedding { text_embedding_tx, image_embedding_tx });
                *weak_ref = Arc::downgrade(&shared_embedding);
                Some(shared_embedding)
            }
        };

        self.shared_embedding = shared_embedding.map(StrongSharedEmbedding);
        self.text_embedding_tx = self.shared_embedding.as_ref().map(|se| se.text_embedding_tx.clone());
        self.image_embedding_tx = self.shared_embedding.as_ref().map(|se| se.image_embedding_tx.clone());

        info!("{} Finished", ctx.id());
        Ok(())
    }
}
