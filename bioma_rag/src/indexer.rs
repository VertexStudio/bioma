use crate::{
    embeddings::{Embeddings, EmbeddingsError, ImageData, StoreEmbeddings},
    markitdown::{AnalyzeMCFile, MarkitDown, MarkitDownError},
    pdf_analyzer::{AnalyzePdf, PdfAnalyzer, PdfAnalyzerError},
    prelude::{Summary, SummaryError},
    summary::{Summarize, SummarizeContent},
};
use base64::Engine;
use bioma_actor::prelude::*;
use derive_more::Display;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use surrealdb::RecordId;
use text_splitter::{ChunkConfig, CodeSplitter, MarkdownSplitter, TextSplitter};
use tracing::{error, info, warn};
use uuid::Uuid;
use walkdir::WalkDir;

use crate::embeddings::EmbeddingContent;

pub const DEFAULT_CHUNK_CAPACITY: std::ops::Range<usize> = 500..2000;
const DEFAULT_CHUNK_OVERLAP: usize = 200;
const DEFAULT_CHUNK_BATCH_SIZE: usize = 50;
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp"];

#[derive(thiserror::Error, Debug)]
pub enum IndexerError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Embeddings error: {0}")]
    Embeddings(#[from] EmbeddingsError),
    #[error("PdfAnalyzer error: {0}")]
    PdfAnalyzer(#[from] PdfAnalyzerError),
    #[error("Markitdown error: {0}")]
    Markitdown(#[from] MarkitDownError),
    #[error("Glob error: {0}")]
    Glob(#[from] glob::GlobError),
    #[error("Pattern error: {0}")]
    Pattern(#[from] glob::PatternError),
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error("Similarity fetch error: {0}")]
    ComputingSimilarity(String),
    #[error("Chunk config error: {0}")]
    ChunkConfig(#[from] text_splitter::ChunkConfigError),
    #[error("Embeddings actor not initialized")]
    EmbeddingsActorNotInitialized,
    #[error("PdfAnalyzer actor not initialized")]
    PdfAnalyzerActorNotInitialized,
    #[error("Markitdown actor not initialized")]
    MarkitdownActorNotInitialized,
    #[error("SurrealDB error: {0}")]
    SurrealDB(#[from] surrealdb::Error),
    #[error("Other error: {0}")]
    Other(String),
    #[error("Summary error: {0}")]
    Summary(#[from] SummaryError),
    #[error("Summary actor not initialized")]
    SummaryActorNotInitialized,
}

impl ActorError for IndexerError {}

#[derive(schemars::JsonSchema, bon::Builder, Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    /// The content to index - can be globs, texts, or images
    #[serde(flatten)]
    pub content: IndexContent,

    /// The source identifier for the indexed content
    #[builder(default = default_source())]
    #[serde(default = "default_source")]
    pub source: String,

    /// Whether to summarize each file or text
    #[builder(default)]
    #[serde(default)]
    pub summarize: bool,
}

#[derive(utoipa::ToSchema, bon::Builder, Debug, Clone, Serialize, Deserialize)]
pub struct ChunkCapacity {
    pub start: usize,
    pub end: usize,
}

#[derive(schemars::JsonSchema, utoipa::ToSchema, bon::Builder, Debug, Clone, Serialize, Deserialize)]
pub struct TextChunkConfig {
    /// Configuration for text chunk size limits
    #[builder(default = default_chunk_capacity())]
    #[serde(default = "default_chunk_capacity")]
    #[schema(value_type = ChunkCapacity)]
    pub chunk_capacity: std::ops::Range<usize>,

    /// The chunk overlap
    #[builder(default = default_chunk_overlap())]
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap: usize,

    /// The chunk batch size
    #[builder(default = default_chunk_batch_size())]
    #[serde(default = "default_chunk_batch_size")]
    pub chunk_batch_size: usize,
}

impl Default for TextChunkConfig {
    fn default() -> Self {
        Self::builder().build()
    }
}

#[derive(schemars::JsonSchema, utoipa::ToSchema, bon::Builder, Debug, Clone, Serialize, Deserialize)]
pub struct GlobsContent {
    /// List of glob patterns
    pub globs: Vec<String>,

    /// Chunk configuration
    #[builder(default)]
    #[serde(default)]
    #[serde(flatten)]
    pub config: TextChunkConfig,
}

#[derive(schemars::JsonSchema, utoipa::ToSchema, bon::Builder, Debug, Clone, Serialize, Deserialize)]
pub struct TextsContent {
    /// The texts to index
    pub texts: Vec<String>,

    /// MIME type for the texts
    #[builder(default = default_text_mime_type())]
    #[serde(default = "default_text_mime_type")]
    pub mime_type: String,

    /// Chunk configuration
    #[builder(default)]
    #[serde(default)]
    #[serde(flatten)]
    pub config: TextChunkConfig,
}

fn default_text_mime_type() -> String {
    "text/plain".to_string()
}

#[derive(schemars::JsonSchema, utoipa::ToSchema, bon::Builder, Debug, Clone, Serialize, Deserialize)]
pub struct ImagesContent {
    /// The base64 encoded images
    pub images: Vec<String>,

    /// Optional MIME type for the images
    #[serde(default)]
    pub mime_type: Option<String>,
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IndexContent {
    /// List of glob patterns to match files for indexing
    #[serde(rename = "globs")]
    Globs(GlobsContent),

    /// List of texts to index
    #[serde(rename = "texts")]
    Texts(TextsContent),

    /// List of base64 encoded images to index
    #[serde(rename = "images")]
    Images(ImagesContent),
}

fn default_source() -> String {
    "/global".to_string()
}

pub fn default_chunk_capacity() -> std::ops::Range<usize> {
    DEFAULT_CHUNK_CAPACITY
}

pub fn default_chunk_overlap() -> usize {
    DEFAULT_CHUNK_OVERLAP
}

pub fn default_chunk_batch_size() -> usize {
    DEFAULT_CHUNK_BATCH_SIZE
}

#[derive(utoipa::ToResponse, utoipa::ToSchema, Debug, Serialize, Deserialize, Clone)]
pub struct IndexedSource {
    pub source: String,
    pub uri: String,
    pub status: IndexStatus,
}

#[derive(utoipa::ToResponse, utoipa::ToSchema, Debug, Serialize, Deserialize, Clone)]
pub enum IndexStatus {
    /// Content has been successfully indexed
    #[schema(title = "IndexedContent")]
    Indexed,

    /// Content was already cached
    #[schema(title = "CachedContent")]
    Cached,

    /// Content failed to be indexed with error message
    #[schema(title = "FailedContent")]
    Failed(String),
}

#[derive(utoipa::ToResponse, utoipa::ToSchema, Debug, Serialize, Deserialize, Clone)]
pub struct Indexed {
    pub indexed: usize,
    pub cached: usize,
    pub sources: Vec<IndexedSource>,
}

#[derive(Display, Debug, Clone, Serialize, Deserialize)]
pub enum CodeLanguage {
    Rust,
    Python,
    Cue,
    Cpp,
    Html,
}

#[derive(Display, Debug, Clone, Serialize, Deserialize)]
pub enum TextType {
    Pdf,
    Markdown,
    MCFile,
    Code(CodeLanguage),
    Text,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextMetadata {
    pub content: TextType,
    pub chunk_number: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageMetadata {
    pub format: String,
    pub dimensions: ImageDimensions,
    pub size_bytes: u64,
    pub modified: u64,
    pub created: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Metadata {
    Text(TextMetadata),
    Image(ImageMetadata),
}

#[derive(Debug)]
enum IndexResult {
    Indexed(Vec<RecordId>, Option<String>),
    Failed,
}

/// The source of the embeddings
#[derive(utoipa::ToSchema, Debug, Serialize, Deserialize, Clone)]
pub struct ContentSource {
    pub source: String,
    pub uri: String,
}

#[derive(utoipa::ToSchema, Debug, Serialize, Deserialize, Clone)]
pub struct DeleteSource {
    pub sources: Vec<String>,

    /// Whether to remove files from disk
    #[serde(default)]
    pub delete_from_disk: bool,
}

#[derive(utoipa::ToSchema, Debug, Serialize, Deserialize, Clone)]
pub struct DeletedSource {
    pub deleted_embeddings: usize,
    pub deleted_sources: Vec<ContentSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageDimensions {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub enum Content {
    Text { content: String, text_type: TextType, chunk_config: (std::ops::Range<usize>, usize, usize) },
    Image { data: ImageContent },
}

#[derive(Debug, Clone)]
pub enum ImageContent {
    Path(String),
    Base64(String, ImageMetadata),
}

impl Indexer {
    /// Checks if a source already exists in the database
    async fn check_source_exists(
        &self,
        ctx: &ActorContext<Self>,
        source: &ContentSource,
    ) -> Result<Option<IndexedSource>, IndexerError> {
        let query = "SELECT id.source AS source, id.uri AS uri FROM source:{source: $source, uri: $uri}".to_string();
        let db = ctx.engine().db();
        let mut results = db
            .lock()
            .await
            .query(&query)
            .bind(("source", source.source.clone()))
            .bind(("uri", source.uri.clone()))
            .await
            .map_err(SystemActorError::from)?;

        let existing_sources: Vec<ContentSource> = results.take(0).map_err(SystemActorError::from)?;
        if !existing_sources.is_empty() {
            Ok(Some(IndexedSource {
                source: source.source.clone(),
                uri: source.uri.clone(),
                status: IndexStatus::Cached,
            }))
        } else {
            Ok(None)
        }
    }

    /// Process summary generation and indexing for a text or image
    ///
    /// This function:
    /// 1. Generates a summary using the Summary actor
    /// 2. Saves the summary to a file
    /// 3. Generates embeddings for the summary
    ///
    /// Returns the IDs of the generated summary embeddings
    async fn process_summary(
        &self,
        ctx: &ActorContext<Self>,
        source: &ContentSource,
        content: &Content,
        summary_id: &ActorId,
        embeddings_id: &ActorId,
        metadata: Option<Vec<Value>>,
    ) -> Result<(String, Vec<RecordId>), IndexerError> {
        // Generate summary based on content type
        let summarize_content = match content {
            Content::Text { content, .. } => SummarizeContent::Text(content.to_string()),
            Content::Image { data } => {
                match data {
                    ImageContent::Path(path) => {
                        // Read image file and convert to base64
                        let image_data = tokio::fs::read(path).await?;
                        let base64_data = base64::engine::general_purpose::STANDARD.encode(image_data);
                        SummarizeContent::Image(base64_data)
                    }
                    ImageContent::Base64(base64_data, _) => SummarizeContent::Image(base64_data.clone()),
                }
            }
        };

        // Generate summary
        let response = ctx
            .send_and_wait_reply::<Summary, Summarize>(
                Summarize { content: summarize_content, uri: source.uri.clone() },
                summary_id,
                SendOptions::builder().timeout(std::time::Duration::from_secs(300)).build(),
            )
            .await?;

        // Generate embeddings for the summary
        let result = ctx
            .send_and_wait_reply::<Embeddings, StoreEmbeddings>(
                StoreEmbeddings { content: EmbeddingContent::Text(vec![response.summary.clone()]), metadata },
                embeddings_id,
                SendOptions::builder().timeout(std::time::Duration::from_secs(200)).build(),
            )
            .await;

        match result {
            Ok(stored_embeddings) => Ok((response.summary, stored_embeddings.ids)),
            Err(e) => {
                error!("Failed to generate summary embeddings: {}", e);
                Ok((response.summary, vec![]))
            }
        }
    }

    /// Process content embeddings and summary in parallel
    ///
    /// This function:
    /// 1. Generates embeddings for the content
    /// 2. If summarization is enabled, generates a summary and its embeddings
    /// 3. Combines all embeddings and returns them with the summary
    async fn process_embeddings_and_summary(
        &self,
        ctx: &ActorContext<Self>,
        source: &ContentSource,
        content: &Content,
        embeddings_id: &ActorId,
        summarize: bool,
        embeddings_content: EmbeddingContent,
        metadata: Option<Vec<Value>>,
    ) -> Result<(Vec<RecordId>, Option<String>), IndexerError> {
        // Start summary generation in parallel only if summarize is enabled
        let summary_future = async {
            let mut summary_embeddings_ids = Vec::new();
            let mut summary_text = None;
            if summarize {
                if let Some(summary_id) = &self.summary_id {
                    match self.process_summary(ctx, source, content, summary_id, embeddings_id, metadata.clone()).await
                    {
                        Ok((summary, ids)) => {
                            summary_text = Some(summary);
                            summary_embeddings_ids.extend(ids);
                        }
                        Err(e) => error!("Failed to process summary: {}", e),
                    }
                }
            }
            (summary_text, summary_embeddings_ids)
        };

        // Process embeddings in parallel with summary generation
        let metadata_clone = metadata.clone();
        let embeddings_future = async {
            let result = ctx
                .send_and_wait_reply::<Embeddings, StoreEmbeddings>(
                    StoreEmbeddings { content: embeddings_content, metadata: metadata_clone },
                    embeddings_id,
                    SendOptions::builder().timeout(std::time::Duration::from_secs(200)).build(),
                )
                .await;

            match result {
                Ok(stored_embeddings) => Ok(stored_embeddings.ids),
                Err(e) => {
                    error!("Failed to generate embeddings: {}", e);
                    Err(IndexerError::System(e))
                }
            }
        };

        // Wait for both operations to complete
        let ((summary_text, summary_ids), embeddings_result) = tokio::join!(summary_future, embeddings_future);

        let mut embeddings_ids = embeddings_result?;

        // Combine original embeddings with summary embeddings
        embeddings_ids.extend(summary_ids);

        Ok((embeddings_ids, summary_text))
    }

    async fn index_content(
        &self,
        ctx: &mut ActorContext<Self>,
        source: ContentSource,
        content: Content,
        embeddings_id: &ActorId,
        summarize: bool,
    ) -> Result<IndexResult, IndexerError> {
        match content {
            Content::Image { data } => {
                let metadata = match &data {
                    ImageContent::Path(path) => {
                        let path_clone = path.clone();
                        tokio::task::spawn_blocking(move || {
                            let file = std::fs::File::open(&path_clone).ok()?;
                            let reader = std::io::BufReader::new(file);
                            let format = image::ImageReader::new(reader).with_guessed_format().ok()?;
                            let format_type = format.format();
                            let dimensions = match format_type {
                                Some(_) => format.into_dimensions().ok()?,
                                None => return None,
                            };

                            let file_metadata = std::fs::metadata(&path_clone).ok()?;

                            let image_metadata = Metadata::Image(ImageMetadata {
                                format: format_type.map(|f| f.extensions_str()[0]).unwrap_or("unknown").to_string(),
                                dimensions: ImageDimensions { width: dimensions.0, height: dimensions.1 },
                                size_bytes: file_metadata.len(),
                                modified: file_metadata
                                    .modified()
                                    .ok()?
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .ok()?
                                    .as_secs(),
                                created: file_metadata
                                    .created()
                                    .ok()?
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .ok()?
                                    .as_secs(),
                            });

                            serde_json::to_value(image_metadata).ok()
                        })
                        .await
                        .map_err(|e| IndexerError::IO(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))?
                    }
                    ImageContent::Base64(_base64_data, image_metadata) => {
                        // We already have the metadata, just convert it to Value
                        Some(
                            serde_json::to_value(Metadata::Image(image_metadata.clone()))
                                .map_err(|e| IndexerError::Other(format!("Failed to serialize metadata: {}", e)))?,
                        )
                    }
                };

                // Convert ImageContent to ImageData for embeddings service
                let image_data = match &data {
                    ImageContent::Path(path) => ImageData::Path(path.clone()),
                    ImageContent::Base64(base64_data, _) => ImageData::Base64(base64_data.clone()),
                };

                let (embeddings_ids, summary_text) = self
                    .process_embeddings_and_summary(
                        ctx,
                        &source,
                        &Content::Image { data: data.clone() },
                        embeddings_id,
                        summarize,
                        EmbeddingContent::Image(vec![image_data]),
                        metadata.map(|m| vec![m]),
                    )
                    .await?;

                if embeddings_ids.is_empty() {
                    Ok(IndexResult::Failed)
                } else {
                    Ok(IndexResult::Indexed(embeddings_ids, summary_text))
                }
            }
            Content::Text { content, text_type, chunk_config: (chunk_capacity, chunk_overlap, chunk_batch_size) } => {
                // Map types if needed
                let (text_type, content) = match text_type {
                    TextType::Code(CodeLanguage::Html) => (TextType::Markdown, mdka::from_html(&content)),
                    TextType::Pdf => (TextType::Markdown, content),
                    TextType::MCFile => (TextType::Markdown, content),
                    _ => (text_type, content),
                };

                let chunks = match &text_type {
                    TextType::Text => {
                        let splitter = TextSplitter::new(
                            ChunkConfig::new(chunk_capacity.clone()).with_trim(false).with_overlap(chunk_overlap)?,
                        );
                        splitter.chunks(&content).collect::<Vec<&str>>()
                    }
                    TextType::Markdown => {
                        let splitter = MarkdownSplitter::new(
                            ChunkConfig::new(chunk_capacity.clone()).with_trim(false).with_overlap(chunk_overlap)?,
                        );
                        splitter.chunks(&content).collect::<Vec<&str>>()
                    }
                    TextType::Code(language) => {
                        let language = match language {
                            CodeLanguage::Rust => tree_sitter_rust::LANGUAGE,
                            CodeLanguage::Python => tree_sitter_python::LANGUAGE,
                            CodeLanguage::Cue => tree_sitter_cue::LANGUAGE,
                            CodeLanguage::Cpp => tree_sitter_cpp::LANGUAGE,
                            CodeLanguage::Html => panic!("Should have been converted to markdown"),
                        };
                        let splitter = CodeSplitter::new(
                            language,
                            ChunkConfig::new(chunk_capacity.clone()).with_trim(false).with_overlap(chunk_overlap)?,
                        )
                        .expect("Invalid tree-sitter language");
                        splitter.chunks(&content).collect::<Vec<&str>>()
                    }
                    _ => panic!("Invalid text type"),
                };

                let chunks = chunks.iter().map(|c| c.to_string()).collect::<Vec<String>>();
                let metadata = chunks
                    .iter()
                    .enumerate()
                    .map(|(i, _)| Metadata::Text(TextMetadata { content: text_type.clone(), chunk_number: i }))
                    .map(|metadata| serde_json::to_value(metadata).unwrap_or_default())
                    .collect::<Vec<Value>>();

                let chunk_batches = chunks.chunks(chunk_batch_size);
                let metadata_batches = metadata.chunks(chunk_batch_size);

                let mut all_embeddings_ids = Vec::new();
                for (chunk_batch, metadata_batch) in chunk_batches.zip(metadata_batches) {
                    let (embeddings_ids, summary_text) = self
                        .process_embeddings_and_summary(
                            ctx,
                            &source,
                            &Content::Text {
                                content: content.clone(),
                                text_type: text_type.clone(),
                                chunk_config: (chunk_capacity.clone(), chunk_overlap, chunk_batch_size),
                            },
                            embeddings_id,
                            summarize && all_embeddings_ids.is_empty(), // Only summarize on first batch
                            EmbeddingContent::Text(chunk_batch.to_vec()),
                            Some(metadata_batch.to_vec()),
                        )
                        .await?;

                    all_embeddings_ids.extend(embeddings_ids);

                    if let Some(text) = summary_text {
                        return Ok(IndexResult::Indexed(all_embeddings_ids, Some(text)));
                    }
                }

                if all_embeddings_ids.is_empty() {
                    Ok(IndexResult::Failed)
                } else {
                    Ok(IndexResult::Indexed(all_embeddings_ids, None))
                }
            }
        }
    }

    /// Handles the result of indexing a source, updating the sources vector and storing the source in the database if needed
    async fn handle_index_result(
        &self,
        ctx: &ActorContext<Self>,
        source: &ContentSource,
        result: Result<IndexResult, IndexerError>,
        sources: &mut Vec<IndexedSource>,
    ) -> Result<bool, IndexerError> {
        match result {
            Ok(IndexResult::Indexed(ids, summary_text)) => {
                if !ids.is_empty() {
                    sources.push(IndexedSource {
                        source: source.source.clone(),
                        uri: source.uri.clone(),
                        status: IndexStatus::Indexed,
                    });

                    let source_query = include_str!("../sql/source.surql");
                    ctx.engine()
                        .db()
                        .lock()
                        .await
                        .query(source_query)
                        .bind(("source", source.source.clone()))
                        .bind(("uri", source.uri.clone()))
                        .bind(("summary", summary_text))
                        .bind(("emb_ids", ids))
                        .bind(("prefix", self.embeddings.table_prefix()))
                        .await
                        .map_err(SystemActorError::from)?;
                    Ok(true)
                } else {
                    sources.push(IndexedSource {
                        source: source.source.clone(),
                        uri: source.uri.clone(),
                        status: IndexStatus::Failed("No embeddings generated".to_string()),
                    });
                    Ok(false)
                }
            }
            Ok(IndexResult::Failed) => {
                sources.push(IndexedSource {
                    source: source.source.clone(),
                    uri: source.uri.clone(),
                    status: IndexStatus::Failed("Failed to generate embeddings".to_string()),
                });
                Ok(false)
            }
            Err(e) => {
                sources.push(IndexedSource {
                    source: source.source.clone(),
                    uri: source.uri.clone(),
                    status: IndexStatus::Failed(format!("Indexing error: {}", e)),
                });
                Ok(false)
            }
        }
    }

    /// Helper function to create a directory for a group of files and return its prefix
    async fn create_group_directory(&self, ctx: &ActorContext<Self>) -> Result<String, IndexerError> {
        let uuid = Uuid::new_v4();
        let uuid_str = uuid.to_string();
        let prefix = &uuid_str[..2];

        let local_store_dir = ctx.engine().local_store_dir();
        let directory = local_store_dir.join("blob").join(prefix);
        tokio::fs::create_dir_all(&directory).await?;

        Ok(prefix.to_string())
    }

    /// Helper function to generate a file path using a shared UUID prefix
    async fn generate_file_path(
        &self,
        ctx: &ActorContext<Self>,
        prefix: &str,
        extension: &str,
    ) -> Result<(String, std::path::PathBuf, std::path::PathBuf), IndexerError> {
        let uuid = Uuid::new_v4();
        let filename = format!("{}.{}", uuid, extension);

        let local_store_dir = ctx.engine().local_store_dir();
        let directory = local_store_dir.join("blob").join(prefix);
        let filepath = directory.join(&filename);
        let uri = format!("blob/{}/{}", prefix, filename);
        let absolute_path = local_store_dir.join(&uri);

        Ok((uri, filepath, absolute_path))
    }

    /// Helper function to save content to file
    async fn save_content_to_file(&self, content: &str, filepath: &Path) -> Result<(), IndexerError> {
        tokio::fs::write(filepath, content).await?;
        Ok(())
    }

    /// Helper function to save base64 image to file
    async fn save_base64_to_file(&self, base64_data: &str, filepath: &Path) -> Result<(), IndexerError> {
        let data = if base64_data.contains(";base64,") {
            base64::engine::general_purpose::STANDARD
                .decode(base64_data.split(";base64,").last().unwrap_or(base64_data))
        } else {
            base64::engine::general_purpose::STANDARD.decode(base64_data)
        }
        .map_err(|e| IndexerError::Other(format!("Base64 decode error: {}", e)))?;

        tokio::fs::write(filepath, data).await?;
        Ok(())
    }
}

impl Message<Index> for Indexer {
    type Response = Indexed;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, message: &Index) -> Result<(), IndexerError> {
        let Some(embeddings_id) = &self.embeddings_id else {
            return Err(IndexerError::EmbeddingsActorNotInitialized);
        };
        let Some(pdf_analyzer_id) = &self.pdf_analyzer_id else {
            return Err(IndexerError::PdfAnalyzerActorNotInitialized);
        };
        let Some(markitdown_id) = &self.markitdown_id else {
            return Err(IndexerError::MarkitdownActorNotInitialized);
        };

        let total_index_time = std::time::Instant::now();
        let mut indexed = 0;
        let mut cached = 0;
        let mut sources = Vec::new();

        match &message.content {
            IndexContent::Globs(GlobsContent { globs, config }) => {
                for pattern in globs {
                    let local_store_dir = ctx.engine().local_store_dir();
                    let full_pattern = if std::path::Path::new(pattern).is_absolute() {
                        pattern.clone()
                    } else {
                        local_store_dir.join(pattern).to_string_lossy().into_owned()
                    };

                    info!("Indexing glob: {}", &full_pattern);
                    let paths = tokio::task::spawn_blocking(move || {
                        let mut paths = Vec::new();
                        for entry in glob::glob(&full_pattern).unwrap().flatten() {
                            if entry.is_file() {
                                paths.push(entry);
                            } else if entry.is_dir() {
                                for entry in WalkDir::new(entry)
                                    .follow_links(true)
                                    .into_iter()
                                    .filter_map(|e| e.ok())
                                    .filter(|e| e.file_type().is_file())
                                {
                                    paths.push(entry.path().to_path_buf());
                                }
                            }
                        }
                        paths
                    })
                    .await;

                    let Ok(paths) = paths else {
                        warn!("Skipping glob: {}", &pattern);
                        sources.push(IndexedSource {
                            source: message.source.clone(),
                            uri: pattern.clone(),
                            status: IndexStatus::Failed("Invalid glob pattern".to_string()),
                        });
                        continue;
                    };

                    for pathbuf in paths {
                        // Convert the full path to a path relative to the local store directory
                        let local_store_dir = ctx.engine().local_store_dir();
                        let relative_path = pathdiff::diff_paths(&pathbuf, local_store_dir)
                            .ok_or_else(|| IndexerError::Other("Failed to get relative path".to_string()))?;
                        let uri = relative_path.to_string_lossy().to_string();
                        let source = ContentSource { source: message.source.clone(), uri: uri.clone() };

                        // Check if source already exists
                        if let Some(indexed_source) = self.check_source_exists(ctx, &source).await? {
                            cached += 1;
                            sources.push(indexed_source);
                            continue;
                        }

                        info!("Indexing path: {}", &pathbuf.display());
                        let ext = pathbuf.extension().and_then(|ext| ext.to_str());

                        let content = if let Some(ext) = ext {
                            if IMAGE_EXTENSIONS.iter().any(|&img_ext| img_ext.eq_ignore_ascii_case(ext)) {
                                Content::Image { data: ImageContent::Path(pathbuf.to_string_lossy().into_owned()) }
                            } else {
                                let chunk_config =
                                    (config.chunk_capacity.clone(), config.chunk_overlap, config.chunk_batch_size);

                                // Special handling for PDF
                                if ext == "pdf" {
                                    match ctx
                                        .send_and_wait_reply::<PdfAnalyzer, AnalyzePdf>(
                                            AnalyzePdf { file_path: pathbuf.clone() },
                                            pdf_analyzer_id,
                                            SendOptions::builder().timeout(std::time::Duration::from_secs(600)).build(),
                                        )
                                        .await
                                    {
                                        Ok(content) => {
                                            Content::Text { content, text_type: TextType::Pdf, chunk_config }
                                        }
                                        Err(e) => {
                                            error!("Failed to convert pdf to md: {}. Error: {}", pathbuf.display(), e);
                                            sources.push(IndexedSource {
                                                source: message.source.clone(),
                                                uri: uri.clone(),
                                                status: IndexStatus::Failed(format!("PDF conversion error: {}", e)),
                                            });
                                            continue;
                                        }
                                    }
                                }
                                // Special handling for MC files
                                else if ext == "docx" || ext == "pptx" || ext == "xlsx" {
                                    match ctx
                                        .send_and_wait_reply::<MarkitDown, AnalyzeMCFile>(
                                            AnalyzeMCFile { file_path: pathbuf.clone() },
                                            markitdown_id,
                                            SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
                                        )
                                        .await
                                    {
                                        Ok(content) => {
                                            Content::Text { content, text_type: TextType::MCFile, chunk_config }
                                        }
                                        Err(e) => {
                                            error!(
                                                "Failed to convert MC file to md: {}. Error: {}",
                                                pathbuf.display(),
                                                e
                                            );
                                            continue;
                                        }
                                    }
                                } else {
                                    // Handle all other text-based files
                                    let text_type = match ext {
                                        "md" => TextType::Markdown,
                                        "rs" => TextType::Code(CodeLanguage::Rust),
                                        "py" => TextType::Code(CodeLanguage::Python),
                                        "cue" => TextType::Code(CodeLanguage::Cue),
                                        "html" => TextType::Code(CodeLanguage::Html),
                                        "cpp" => TextType::Code(CodeLanguage::Cpp),
                                        "h" => TextType::Code(CodeLanguage::Cpp),
                                        _ => TextType::Text,
                                    };

                                    match tokio::fs::read_to_string(&pathbuf).await {
                                        Ok(content) => Content::Text { content, text_type, chunk_config },
                                        Err(_) => continue,
                                    }
                                }
                            }
                        } else {
                            // Handle files without extensions as text
                            let chunk_config =
                                (config.chunk_capacity.clone(), config.chunk_overlap, config.chunk_batch_size);
                            match tokio::fs::read_to_string(&pathbuf).await {
                                Ok(content) => Content::Text { content, text_type: TextType::Text, chunk_config },
                                Err(_) => continue,
                            }
                        };

                        // Process content
                        let result =
                            self.index_content(ctx, source.clone(), content, embeddings_id, message.summarize).await;
                        if self.handle_index_result(ctx, &source, result, &mut sources).await? {
                            indexed += 1;
                        }
                    }
                }
            }
            IndexContent::Texts(TextsContent { texts, mime_type, config }) => {
                // Create a shared directory for all texts in this message
                let prefix = self.create_group_directory(ctx).await?;

                // Determine extension based on MIME type once for all texts
                let extension = match mime_type.split('/').last() {
                    Some("plain") => "txt",
                    Some("markdown") | Some("md") => "md",
                    Some("html") => "html",
                    Some(ext) if !ext.is_empty() => ext,
                    _ => "txt", // Default to txt if MIME type is invalid
                }
                .to_string();

                for text in texts {
                    let (uri, filepath, _) = self.generate_file_path(ctx, &prefix, &extension).await?;
                    let source = ContentSource { source: message.source.clone(), uri };

                    // Check if source already exists
                    if let Some(indexed_source) = self.check_source_exists(ctx, &source).await? {
                        cached += 1;
                        sources.push(indexed_source);
                        continue;
                    }

                    // Save the text content to file
                    self.save_content_to_file(text, &filepath).await?;

                    info!("Indexing text: {}", &filepath.display());
                    let content = Content::Text {
                        content: text.clone(),
                        text_type: TextType::Text,
                        chunk_config: (config.chunk_capacity.clone(), config.chunk_overlap, config.chunk_batch_size),
                    };

                    // Process content
                    let result =
                        self.index_content(ctx, source.clone(), content, embeddings_id, message.summarize).await;
                    if self.handle_index_result(ctx, &source, result, &mut sources).await? {
                        indexed += 1;
                    }
                }
            }
            IndexContent::Images(ImagesContent { images, mime_type }) => {
                // Create a shared directory for all images in this message
                let prefix = self.create_group_directory(ctx).await?;

                // If MIME type is provided, determine the extension once for all images
                let extension_from_mime = mime_type.as_ref().and_then(|mime| match mime.split('/').last() {
                    Some("jpeg") => Some("jpg".to_string()),
                    Some(ext) if !ext.is_empty() => Some(ext.to_string()),
                    _ => None,
                });

                for image in images {
                    // Gotta get metadata early because we need to know the extension to save the file
                    let (extension, image_metadata) = tokio::task::spawn_blocking({
                        let image_clone = image.clone();
                        let extension_from_mime = extension_from_mime.clone();
                        move || {
                            // Decode base64 data
                            let base64_content = if let Some(idx) = image_clone.find(";base64,") {
                                &image_clone[idx + 8..]
                            } else {
                                &image_clone
                            };

                            let image_data = match base64::engine::general_purpose::STANDARD.decode(base64_content) {
                                Ok(data) => data,
                                Err(_) => return Err("Invalid base64 data"),
                            };

                            let cursor = std::io::Cursor::new(&image_data);
                            let format = match image::ImageReader::new(cursor).with_guessed_format() {
                                Ok(format) => format,
                                Err(_) => return Err("Failed to determine image format"),
                            };

                            let format_type = format.format();
                            let dimensions = match format_type {
                                Some(_) => match format.into_dimensions() {
                                    Ok(dim) => dim,
                                    Err(_) => return Err("Failed to get image dimensions"),
                                },
                                None => return Err("Unknown image format"),
                            };

                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();

                            // Use the extension from MIME type if available, otherwise determine from image data
                            let extension = extension_from_mime.unwrap_or_else(|| {
                                format_type.map(|f| f.extensions_str()[0]).unwrap_or("unknown").to_string()
                            });

                            let metadata = ImageMetadata {
                                format: extension.clone(),
                                dimensions: ImageDimensions { width: dimensions.0, height: dimensions.1 },
                                size_bytes: image_data.len() as u64,
                                modified: now,
                                created: now,
                            };

                            Ok((extension, metadata))
                        }
                    })
                    .await
                    .map_err(|e| IndexerError::IO(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))?
                    .map_err(|e| IndexerError::Other(format!("Failed to process image: {}", e)))?;

                    let (uri, filepath, _) = self.generate_file_path(ctx, &prefix, &extension).await?;
                    let source = ContentSource { source: message.source.clone(), uri: uri.clone() };

                    // Check if source already exists
                    if let Some(indexed_source) = self.check_source_exists(ctx, &source).await? {
                        cached += 1;
                        sources.push(indexed_source);
                        continue;
                    }

                    // Save the image content to file
                    self.save_base64_to_file(image, &filepath).await?;

                    info!("Indexing image: {}", &filepath.display());
                    let content = Content::Image { data: ImageContent::Base64(image.clone(), image_metadata) };

                    // Process content
                    let result =
                        self.index_content(ctx, source.clone(), content, embeddings_id, message.summarize).await;
                    if self.handle_index_result(ctx, &source, result, &mut sources).await? {
                        indexed += 1;
                    }
                }
            }
        }

        info!("Indexed {} paths, cached {} paths, in {:?}", indexed, cached, total_index_time.elapsed());
        ctx.reply(Indexed { indexed, cached, sources }).await?;
        Ok(())
    }
}

impl Message<DeleteSource> for Indexer {
    type Response = DeletedSource;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, message: &DeleteSource) -> Result<(), IndexerError> {
        let query = include_str!("../sql/del_source.surql").replace("{prefix}", &self.embeddings.table_prefix());
        let db = ctx.engine().db();

        let mut results = db
            .lock()
            .await
            .query(&query)
            .bind(("sources", message.sources.clone()))
            .await
            .map_err(SystemActorError::from)?;

        let delete_result: DeletedSource = results
            .take::<Vec<DeletedSource>>(0)
            .map_err(IndexerError::from)?
            .pop()
            .ok_or(IndexerError::Other("No delete result found".to_string()))?;

        // Process file deletions only if delete_from_disk is true
        if message.delete_from_disk {
            for source in &delete_result.deleted_sources {
                let local_store_dir = ctx.engine().local_store_dir();
                let source_path = local_store_dir.join(&source.uri);
                if source_path.exists() {
                    if source_path.is_dir() {
                        tokio::fs::remove_dir_all(&source_path).await.ok();
                    } else {
                        tokio::fs::remove_file(&source_path).await.ok();
                    }
                }
            }
        }

        ctx.reply(delete_result).await?;
        Ok(())
    }
}

#[derive(bon::Builder, Debug, Serialize, Deserialize, Default)]
pub struct Indexer {
    pub embeddings: Embeddings,
    pub pdf_analyzer: PdfAnalyzer,
    pub markitdown: MarkitDown,
    pub summary: Summary,
    embeddings_id: Option<ActorId>,
    pdf_analyzer_id: Option<ActorId>,
    markitdown_id: Option<ActorId>,
    summary_id: Option<ActorId>,
    #[serde(skip)]
    pdf_analyzer_handle: Option<tokio::task::JoinHandle<()>>,
    #[serde(skip)]
    embeddings_handle: Option<tokio::task::JoinHandle<()>>,
    #[serde(skip)]
    markitdown_handle: Option<tokio::task::JoinHandle<()>>,
    #[serde(skip)]
    summary_handle: Option<tokio::task::JoinHandle<()>>,
}

impl Actor for Indexer {
    type Error = IndexerError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), IndexerError> {
        self.init(ctx).await?;

        // Start the message stream
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(input) = frame.is::<Index>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            } else if let Some(input) = frame.is::<DeleteSource>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            }
        }

        Ok(())
    }
}

impl Indexer {
    pub async fn init(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), IndexerError> {
        // Used to namespace child actors
        let self_id = ctx.id().clone();

        // Generate child id for pdf-analyzer
        let pdf_analyzer_id = ActorId::of::<PdfAnalyzer>(format!("{}/pdf-analyzer", self_id.name()));
        self.pdf_analyzer_id = Some(pdf_analyzer_id.clone());

        // Spawn the pdf-analyzer actor
        let (mut pdf_analyzer_ctx, mut pdf_analyzer_actor) = Actor::spawn(
            ctx.engine().clone(),
            pdf_analyzer_id.clone(),
            self.pdf_analyzer.clone(),
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;

        // Start the pdf-analyzer actor
        let pdf_analyzer_handle = tokio::spawn(async move {
            if let Err(e) = pdf_analyzer_actor.start(&mut pdf_analyzer_ctx).await {
                error!("PdfAnalyzer actor error: {}", e);
            }
        });

        // Generate child id for embeddings
        let embeddings_id = ActorId::of::<Embeddings>(format!("{}/embeddings", self_id.name()));
        self.embeddings_id = Some(embeddings_id.clone());

        // Spawn the embeddings actor
        let (mut embeddings_ctx, mut embeddings_actor) = Actor::spawn(
            ctx.engine().clone(),
            embeddings_id.clone(),
            self.embeddings.clone(),
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;

        // Start the embeddings actor
        let embeddings_handle = tokio::spawn(async move {
            if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
                error!("Embeddings actor error: {}", e);
            }
        });

        // Generate child id for markitdown
        let markitdown_id = ActorId::of::<MarkitDown>(format!("{}/markitdown", self_id.name()));
        self.markitdown_id = Some(markitdown_id.clone());

        let (mut markitdown_ctx, mut markitdown_actor) = Actor::spawn(
            ctx.engine().clone(),
            markitdown_id.clone(),
            self.markitdown.clone(),
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;

        let markitdown_handle = tokio::spawn(async move {
            if let Err(e) = markitdown_actor.start(&mut markitdown_ctx).await {
                error!("MarkitDown actor error: {}", e);
            }
        });

        // Generate child id for summary
        let summary_id = ActorId::of::<Summary>(format!("{}/summary", self_id.name()));
        self.summary_id = Some(summary_id.clone());

        // Spawn the summary actor
        let (mut summary_ctx, mut summary_actor) = Actor::spawn(
            ctx.engine().clone(),
            summary_id.clone(),
            self.summary.clone(),
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;

        // Start the summary actor
        let summary_handle = tokio::spawn(async move {
            if let Err(e) = summary_actor.start(&mut summary_ctx).await {
                error!("Summary actor error: {}", e);
            }
        });

        self.pdf_analyzer_handle = Some(pdf_analyzer_handle);
        self.embeddings_handle = Some(embeddings_handle);
        self.markitdown_handle = Some(markitdown_handle);
        self.summary_handle = Some(summary_handle);

        info!("Indexer ready");

        Ok(())
    }
}
