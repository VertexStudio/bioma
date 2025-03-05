use crate::indexer::ContentSource;
use base64::Engine as _;
use bioma_actor::prelude::*;
use bon::Builder;
use derive_more::{Deref, Display};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use surrealdb::value::RecordId;
use tempfile::Builder as TempBuilder;
use tokio::sync::Mutex;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{error, info};

lazy_static! {
    static ref SHARED_EMBEDDINGS: Arc<Mutex<HashMap<Model, Weak<SharedEmbedding>>>> =
        Arc::new(Mutex::new(HashMap::new()));
}

struct SharedEmbedding {
    embedding_tx: mpsc::Sender<EmbeddingRequest>,
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
    SendTextEmbeddings(#[from] mpsc::error::SendError<EmbeddingRequest>),
    #[error("Error receiving text embeddings: {0}")]
    RecvEmbeddings(#[from] oneshot::error::RecvError),
    #[error("Base64 decode error: {0}")]
    Base64Decode(#[from] base64::DecodeError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Image format error: {0}")]
    ImageFormat(String),
    #[error("Persist error: {0}")]
    Persist(#[from] tempfile::PersistError),
    #[error("Input size too large: {0} tokens (max: {1})")]
    InputSizeTooLarge(usize, usize),
}

impl ActorError for EmbeddingsError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImageData {
    Path(String),
    Base64(String),
}

impl ImageData {
    fn parse_base64_image(base64_str: &str) -> Result<(Vec<u8>, Option<String>), EmbeddingsError> {
        // Handle both raw base64 and data URLs
        let (base64_data, format) = if let Some(idx) = base64_str.find("data:image/") {
            // Parse data URL format
            let format_end = base64_str[idx + 11..]
                .find(';')
                .map(|i| idx + 11 + i)
                .ok_or_else(|| EmbeddingsError::ImageFormat("Invalid data URL format".into()))?;

            let format = base64_str[idx + 11..format_end].to_string();

            let base64_start = base64_str[format_end..]
                .find("base64,")
                .map(|i| format_end + i + 7)
                .ok_or_else(|| EmbeddingsError::ImageFormat("Missing base64 marker".into()))?;

            (&base64_str[base64_start..], Some(format))
        } else {
            (base64_str, None)
        };

        // Decode base64
        let image_data =
            base64::engine::general_purpose::STANDARD.decode(base64_data).map_err(EmbeddingsError::Base64Decode)?;

        Ok((image_data, format))
    }

    fn determine_image_extension(data: &[u8], format_hint: Option<String>) -> Result<&'static str, EmbeddingsError> {
        // First try the format hint if provided
        if let Some(format) = format_hint {
            match format.to_lowercase().as_str() {
                "jpeg" | "jpg" => return Ok(".jpg"),
                "png" => return Ok(".png"),
                "webp" => return Ok(".webp"),
                _ => {} // Fall through to signature detection
            }
        }

        // Check file signature
        if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Ok(".jpg")
        } else if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
            Ok(".png")
        } else if data.starts_with(&[0x52, 0x49, 0x46, 0x46]) && data[8..12] == [0x57, 0x45, 0x42, 0x50] {
            Ok(".webp")
        } else {
            Err(EmbeddingsError::ImageFormat("Unsupported image format".into()))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EmbeddingContent {
    Text(Vec<String>),
    Image(Vec<ImageData>),
}

impl EmbeddingContent {
    fn process_image_data(images: &[ImageData]) -> Result<Vec<PathBuf>, EmbeddingsError> {
        images
            .iter()
            .map(|image_data| match image_data {
                ImageData::Path(path) => Ok(PathBuf::from(path)),
                ImageData::Base64(base64_str) => {
                    // Parse and validate base64 data
                    let (image_data, format_hint) = ImageData::parse_base64_image(base64_str)?;

                    // Determine file extension
                    let extension = ImageData::determine_image_extension(&image_data, format_hint)?;

                    // Create temporary file with correct extension
                    let mut temp_file = TempBuilder::new().prefix("embedding_").suffix(extension).tempfile()?;

                    // Write image data
                    temp_file.write_all(&image_data)?;

                    // Get path and persist the file
                    let (file, path) = temp_file.keep()?;
                    std::mem::forget(file);

                    Ok(path)
                }
            })
            .collect()
    }
}

pub struct EmbeddingRequest {
    response_tx: oneshot::Sender<Result<Vec<Vec<f32>>, fastembed::Error>>,
    content: EmbeddingContent,
}

/// Store embeddings for texts or images
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreEmbeddings {
    /// The content to embed (either texts or images)
    pub content: EmbeddingContent,
    /// Metadata to store with the embeddings
    pub metadata: Option<Vec<Value>>,
}

/// Generate embeddings for texts or images
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateEmbeddings {
    /// The content to embed (either texts or images)
    pub content: EmbeddingContent,
}

/// The generated embeddings
#[derive(utoipa::ToSchema, Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedEmbeddings {
    pub embeddings: Vec<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredEmbeddings {
    pub ids: Vec<RecordId>,
}

/// The query to search for
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Query {
    Embedding(Vec<f32>),
    Text(String),
    Image(ImageData),
}

/// Get the top k similar embeddings to a query
#[derive(Builder, Debug, Clone, Serialize, Deserialize)]
pub struct TopK {
    /// The query to search for
    pub query: Query,
    /// A list of sources to filter the search
    #[builder(default = default_sources())]
    pub sources: Vec<String>,
    /// Number of similar embeddings to return
    pub k: usize,
    /// The threshold for the similarity score
    pub threshold: f32,
}

fn default_sources() -> Vec<String> {
    vec!["/global".to_string()]
}

/// The similarity between a query and an embedding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Similarity {
    pub text: Option<String>,
    pub similarity: f32,
    pub source: Option<ContentSource>,
    pub metadata: Option<Value>,
}

#[derive(bon::Builder, Debug, Serialize, Deserialize)]
pub struct Embeddings {
    pub table_name_prefix: Option<String>,
    #[builder(default = Model::NomicEmbedTextV15)]
    pub model: Model,
    #[builder(default = ImageModel::NomicEmbedVisionV15)]
    pub image_model: ImageModel,
    #[serde(skip)]
    embedding_tx: Option<mpsc::Sender<EmbeddingRequest>>,
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
            table_name_prefix: self.table_name_prefix.clone(),
            model: self.model.clone(),
            image_model: self.image_model.clone(),
            embedding_tx: None,
            shared_embedding: None,
        }
    }
}

impl Default for Embeddings {
    fn default() -> Self {
        Self::builder().build()
    }
}

impl Message<TopK> for Embeddings {
    type Response = Vec<Similarity>;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, message: &TopK) -> Result<(), EmbeddingsError> {
        let query_embedding = match &message.query {
            Query::Embedding(embedding) => embedding.clone(),
            Query::Text(text) => {
                let Some(embedding_tx) = self.embedding_tx.as_ref() else {
                    return Err(EmbeddingsError::TextEmbeddingNotInitialized);
                };
                let (tx, rx) = oneshot::channel();
                embedding_tx
                    .send(EmbeddingRequest { response_tx: tx, content: EmbeddingContent::Text(vec![text.to_string()]) })
                    .await?;
                rx.await??.first().cloned().ok_or(EmbeddingsError::NoEmbeddingsGenerated)?
            }
            Query::Image(image_data) => {
                let Some(embedding_tx) = self.embedding_tx.as_ref() else {
                    return Err(EmbeddingsError::TextEmbeddingNotInitialized);
                };
                let (tx, rx) = oneshot::channel();
                embedding_tx
                    .send(EmbeddingRequest {
                        response_tx: tx,
                        content: EmbeddingContent::Image(vec![image_data.clone()]),
                    })
                    .await?;
                rx.await??.first().cloned().ok_or(EmbeddingsError::NoEmbeddingsGenerated)?
            }
        };

        let db = ctx.engine().db();
        let query_sql = include_str!("../sql/similarities.surql")
            .replace("{top_k}", &message.k.to_string())
            .replace("{prefix}", &self.table_prefix());

        let mut results = db
            .lock()
            .await
            .query(query_sql)
            .bind(("query", query_embedding))
            .bind(("threshold", message.threshold))
            .bind(("sources", message.sources.clone()))
            .bind(("prefix", self.table_prefix()))
            .await
            .map_err(SystemActorError::from)?;
        let results: Vec<Similarity> = results.take(0).map_err(SystemActorError::from)?;
        ctx.reply(results).await?;
        Ok(())
    }
}

impl Message<StoreEmbeddings> for Embeddings {
    type Response = StoredEmbeddings;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, message: &StoreEmbeddings) -> Result<(), EmbeddingsError> {
        let Some(embedding_tx) = self.embedding_tx.as_ref() else {
            return Err(EmbeddingsError::TextEmbeddingNotInitialized);
        };

        let (tx, rx) = oneshot::channel();
        embedding_tx.send(EmbeddingRequest { response_tx: tx, content: message.content.clone() }).await?;

        let embeddings = rx.await??;

        let db = ctx.engine().db();
        let emb_query = include_str!("../sql/embeddings.surql");

        let mut embeddings_ids: Vec<RecordId> = Vec::new();

        // Store embeddings
        for (i, embedding) in embeddings.iter().enumerate() {
            let embedding = embedding.clone();
            let metadata = message.metadata.as_ref().map(|m| m[i].clone()).unwrap_or(Value::Null);
            let model_id = match &message.content {
                EmbeddingContent::Text(_) => RecordId::from_table_key("model", self.model.to_string()),
                EmbeddingContent::Image(_) => RecordId::from_table_key("model", self.image_model.to_string()),
            };

            let text = match &message.content {
                EmbeddingContent::Text(texts) => Some(texts[i].clone()),
                EmbeddingContent::Image(_) => None,
            };

            let mut results = db
                .lock()
                .await
                .query(emb_query)
                .bind(("embedding", embedding))
                .bind(("metadata", metadata))
                .bind(("model_id", model_id))
                .bind(("prefix", self.table_prefix()))
                .bind(("text", text))
                .await
                .map_err(SystemActorError::from)?;

            let id: Result<Option<RecordId>, _> = results.take(3).map_err(SystemActorError::from);
            if let Some(id) = id.map_err(EmbeddingsError::from)? {
                embeddings_ids.push(id);
            } else {
                return Err(EmbeddingsError::NoEmbeddingsGenerated);
            }
        }

        ctx.reply(StoredEmbeddings { ids: embeddings_ids }).await?;
        Ok(())
    }
}

impl Message<GenerateEmbeddings> for Embeddings {
    type Response = GeneratedEmbeddings;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: &GenerateEmbeddings,
    ) -> Result<(), EmbeddingsError> {
        let Some(embedding_tx) = self.embedding_tx.as_ref() else {
            return Err(EmbeddingsError::TextEmbeddingNotInitialized);
        };

        let (tx, rx) = oneshot::channel();
        embedding_tx.send(EmbeddingRequest { response_tx: tx, content: message.content.clone() }).await?;

        let embeddings = rx.await??;
        ctx.reply(GeneratedEmbeddings { embeddings }).await?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Display, Eq, Hash, PartialEq)]
pub enum Model {
    NomicEmbedTextV15,
    ClipVitB32Text,
}

fn get_fastembed_model(model: &Model) -> fastembed::EmbeddingModel {
    match model {
        Model::NomicEmbedTextV15 => fastembed::EmbeddingModel::NomicEmbedTextV15,
        Model::ClipVitB32Text => fastembed::EmbeddingModel::ClipVitB32,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Display)]
pub enum ImageModel {
    NomicEmbedVisionV15,
    ClipVitB32Vision,
}

fn get_fastembed_image_model(model: &ImageModel) -> fastembed::ImageEmbeddingModel {
    match model {
        ImageModel::ClipVitB32Vision => fastembed::ImageEmbeddingModel::ClipVitB32,
        ImageModel::NomicEmbedVisionV15 => fastembed::ImageEmbeddingModel::NomicEmbedVisionV15,
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
            if let Some(input) = frame.is::<StoreEmbeddings>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            } else if let Some(input) = frame.is::<GenerateEmbeddings>() {
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

impl Embeddings {
    const MAX_TOTAL_INPUT_LENGTH: usize = 81_920;

    pub async fn init(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), EmbeddingsError> {
        info!("{} Started", ctx.id());

        // Manage a shared embedding task
        let shared_embedding = {
            let mut embeddings_map = SHARED_EMBEDDINGS.lock().await;
            let existing_embedding = if let Some(weak_ref) = embeddings_map.get(&self.model) {
                if let Some(strong_ref) = weak_ref.upgrade() {
                    // Return the existing shared embedding
                    Some(strong_ref)
                } else {
                    // Remove the expired weak reference
                    embeddings_map.remove(&self.model);
                    None
                }
            } else {
                None
            };

            if let Some(embedding) = existing_embedding {
                embedding
            } else {
                // Create new shared embedding
                let ctx_id = ctx.id().clone();

                // Get text model info
                let fastembed_model = get_fastembed_model(&self.model);
                let text_model_info = fastembed::TextEmbedding::get_model_info(&fastembed_model)?;
                let text_model_info = ModelInfo {
                    name: self.model.clone(),
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

                // Assert that all models dimensions are the same
                if text_model_info.dim != image_model_info.dim {
                    panic!("Text and image model dimensions must be the same");
                }

                // Assert the model pairs are valid
                match (&self.model, &self.image_model) {
                    (Model::ClipVitB32Text, ImageModel::ClipVitB32Vision) => {}
                    (Model::NomicEmbedTextV15, ImageModel::NomicEmbedVisionV15) => {}
                    _ => panic!("Invalid model pair"),
                }

                // Define schema
                let schema_def = include_str!("../sql/def.surql")
                    .replace("{prefix}", &self.table_prefix())
                    .replace("{dim}", &text_model_info.dim.to_string());

                // Execute the schema definition
                let db = ctx.engine().db();
                db.lock().await.query(&schema_def).await.map_err(SystemActorError::from)?;

                // Store text model info in database if not already present
                let model: Result<Option<Record>, _> = db
                    .lock()
                    .await
                    .create(("model", self.model.to_string()))
                    .content(text_model_info)
                    .await
                    .map_err(SystemActorError::from);
                if let Ok(Some(model)) = &model {
                    info!("Model {:?} stored with id: {}", self.model, model.id);
                }

                // Store image model info in database if not already present
                let model: Result<Option<Record>, _> = db
                    .lock()
                    .await
                    .create(("model", self.image_model.to_string()))
                    .content(image_model_info)
                    .await
                    .map_err(SystemActorError::from);
                if let Ok(Some(model)) = &model {
                    info!("Model {:?} stored with id: {}", self.image_model, model.id);
                }

                // Create a new shared embedding
                let (embedding_tx, mut embedding_rx) = mpsc::channel::<EmbeddingRequest>(100);
                let text_model = self.model.clone();
                let image_model = self.image_model.clone();
                let cache_dir = ctx.engine().huggingface_cache_dir().clone();

                let _embedding_task: JoinHandle<Result<(), fastembed::Error>> =
                    tokio::task::spawn_blocking(move || {
                        // Initialize both text and image embeddings
                        let mut text_options = fastembed::InitOptions::new(get_fastembed_model(&text_model))
                            .with_cache_dir(cache_dir.clone());
                        let mut image_options =
                            fastembed::ImageInitOptions::new(get_fastembed_image_model(&image_model))
                                .with_cache_dir(cache_dir);

                        #[cfg(target_os = "macos")]
                        {
                            text_options = text_options.with_execution_providers(vec![
                                ort::execution_providers::CoreMLExecutionProvider::default().build(),
                            ]);

                            image_options = image_options.with_execution_providers(vec![
                                ort::execution_providers::CoreMLExecutionProvider::default().build(),
                            ]);
                        }

                        #[cfg(target_os = "linux")]
                        {
                            text_options = text_options.with_execution_providers(vec![
                                ort::execution_providers::CUDAExecutionProvider::default().build(),
                            ]);

                            image_options = image_options.with_execution_providers(vec![
                                ort::execution_providers::CUDAExecutionProvider::default().build(),
                            ]);
                        }

                        let text_embedding = fastembed::TextEmbedding::try_new(text_options)?;
                        let image_embedding = fastembed::ImageEmbedding::try_new(image_options)?;

                        while let Some(request) = embedding_rx.blocking_recv() {
                            let start = std::time::Instant::now();

                            match request.content {
                                EmbeddingContent::Text(texts) => {
                                    let text_count = texts.len();
                                    let mut all_embeddings = Vec::with_capacity(text_count);

                                    // Calculate total input size across all texts
                                    let total_input_length: usize = texts.iter().map(|text| text.len()).sum();

                                    // Prevent GPU memory overload by limiting the total size of text that can be processed at once
                                    if total_input_length > Self::MAX_TOTAL_INPUT_LENGTH {
                                        error!(
                                            "Total text input size too large: {} characters (max: {})",
                                            total_input_length,
                                            Self::MAX_TOTAL_INPUT_LENGTH
                                        );

                                        let error = EmbeddingsError::InputSizeTooLarge(
                                            total_input_length,
                                            Self::MAX_TOTAL_INPUT_LENGTH,
                                        );

                                        let fastembed_error = fastembed::Error::msg(error);
                                        let _ = request.response_tx.send(Err(fastembed_error));

                                        continue;
                                    }

                                    // Process texts in chunks
                                    for chunk in texts.chunks(10) {
                                        let avg_text_len = chunk.iter().map(|text| text.len() as f32).sum::<f32>()
                                            / chunk.len() as f32;
                                        match text_embedding.embed(chunk.to_vec(), None) {
                                            Ok(mut embeddings) => {
                                                info!(
                                                    "Generated {} text embeddings (avg. {:.1} chars) in {:?}",
                                                    chunk.len(),
                                                    avg_text_len,
                                                    start.elapsed()
                                                );
                                                all_embeddings.append(&mut embeddings);
                                            }
                                            Err(err) => {
                                                error!("Failed to generate text embeddings: {}", err);
                                                let _ = request.response_tx.send(Err(err));
                                                return Ok(());
                                            }
                                        }
                                    }

                                    let _ = request.response_tx.send(Ok(all_embeddings));
                                }
                                EmbeddingContent::Image(images) => {
                                    let image_count = images.len();
                                    match EmbeddingContent::process_image_data(&images) {
                                        Ok(paths) => match image_embedding.embed(paths, None) {
                                            Ok(embeddings) => {
                                                info!(
                                                    "Generated {} image embeddings in {:?}",
                                                    image_count,
                                                    start.elapsed()
                                                );
                                                let _ = request.response_tx.send(Ok(embeddings));
                                            }
                                            Err(err) => {
                                                error!("Failed to generate image embeddings: {}", err);
                                                let _ = request.response_tx.send(Err(err));
                                            }
                                        },
                                        Err(err) => {
                                            error!("Failed to process image data: {}", err);
                                            let _ = request.response_tx.send(Err(err.into()));
                                        }
                                    }
                                }
                            }
                        }

                        info!("{} embedding task finished", ctx_id);
                        Ok(())
                    });

                // Store the shared embedding
                let shared_embedding = Arc::new(SharedEmbedding { embedding_tx });
                embeddings_map.insert(self.model.clone(), Arc::downgrade(&shared_embedding));
                shared_embedding
            }
        };

        let shared_embedding = Some(shared_embedding); // Wrap in Option first
        self.shared_embedding = shared_embedding.map(StrongSharedEmbedding);
        self.embedding_tx = self.shared_embedding.as_ref().map(|se| se.embedding_tx.clone());

        info!("{} Finished", ctx.id());
        Ok(())
    }

    pub fn table_prefix(&self) -> String {
        self.table_name_prefix.as_ref().unwrap_or(&self.model.to_string()).clone()
    }
}
