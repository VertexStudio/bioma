use crate::{
    embeddings::{Embeddings, EmbeddingsError, Image, StoreImageEmbeddings, StoreTextEmbeddings},
    pdf_analyzer::{AnalyzePdf, PdfAnalyzer, PdfAnalyzerError},
};
use bioma_actor::prelude::*;
use derive_more::Display;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::Value;
use std::borrow::Cow;
use text_splitter::{ChunkConfig, CodeSplitter, MarkdownSplitter, TextSplitter};
use tracing::{debug, error, info, warn};
use walkdir::WalkDir;

const DEFAULT_INDEXER_TAG: &str = "indexer_content";
const DEFAULT_CHUNK_CAPACITY: std::ops::Range<usize> = 500..2000;
const DEFAULT_CHUNK_OVERLAP: usize = 200;
const DEFAULT_CHUNK_BATCH_SIZE: usize = 50;

#[derive(thiserror::Error, Debug)]
pub enum IndexerError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Embeddings error: {0}")]
    Embeddings(#[from] EmbeddingsError),
    #[error("PdfAnalyzer error: {0}")]
    PdfAnalyzer(#[from] PdfAnalyzerError),
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
    #[error("Image embedding error: {0}")]
    ImageEmbedding(String),
}

impl ActorError for IndexerError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourcedText {
    pub source: String,
    pub text: String,
    pub text_type: TextType,
}

#[derive(bon::Builder, Debug, Clone, Serialize, Deserialize)]
pub struct IndexTexts {
    pub texts: Vec<SourcedText>,
    #[builder(default = default_chunk_capacity())]
    #[serde(default = "default_chunk_capacity")]
    pub chunk_capacity: std::ops::Range<usize>,
    #[builder(default = default_chunk_overlap())]
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap: usize,
    #[builder(default = default_chunk_batch_size())]
    #[serde(default = "default_chunk_batch_size")]
    pub chunk_batch_size: usize,
}

#[derive(bon::Builder, Debug, Clone, Serialize, Deserialize)]
pub struct IndexGlobs {
    pub globs: Vec<String>,
    #[builder(default = default_chunk_capacity())]
    #[serde(default = "default_chunk_capacity")]
    pub chunk_capacity: std::ops::Range<usize>,
    #[builder(default = default_chunk_overlap())]
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap: usize,
    #[builder(default = default_chunk_batch_size())]
    #[serde(default = "default_chunk_batch_size")]
    pub chunk_batch_size: usize,
}

#[derive(bon::Builder, Debug, Clone, Serialize, Deserialize)]
pub struct IndexImages {
    /// The images to index
    pub images: Vec<Image>,
}

fn default_chunk_capacity() -> std::ops::Range<usize> {
    DEFAULT_CHUNK_CAPACITY
}

fn default_chunk_overlap() -> usize {
    DEFAULT_CHUNK_OVERLAP
}

fn default_chunk_batch_size() -> usize {
    DEFAULT_CHUNK_BATCH_SIZE
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Indexed {
    pub indexed: usize,
    pub cached: usize,
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
    Code(CodeLanguage),
    Text,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkMetadata {
    pub source: String,
    pub text_type: TextType,
    pub chunk_number: usize,
    pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SourceEmbeddings {
    pub source: String,
    pub uris: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SourceImageEmbeddings {
    pub source: String,
}

#[derive(Debug)]
enum IndexResult {
    Indexed(usize),
    Cached,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteSource {
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletedSource {
    pub deleted_embeddings: usize,
    pub deleted_sources: Vec<String>,
    pub not_found_sources: Vec<String>,
}

impl Indexer {
    async fn index_text(
        &self,
        ctx: &mut ActorContext<Self>,
        source: String,
        uri: String,
        content: String,
        text_type: TextType,
        chunk_capacity: std::ops::Range<usize>,
        chunk_overlap: usize,
        chunk_batch_size: usize,
        embeddings_id: &ActorId,
    ) -> Result<IndexResult, IndexerError> {
        // TODO: This query is twice slow compared to the previous one. Guess it's because we're ->source_text_embeddings.out.metadata.uri twice. Not sure.
        let source_embeddings = ctx
            .engine()
            .db()
            .query("SELECT source, ->source_text_embeddings.out.metadata.uri AS uris FROM source:{source:$source, tag:$tag} WHERE ->source_text_embeddings.out.metadata.uri CONTAINS $uri")
            .bind(("source", source.clone()))
            .bind(("tag", self.tag.clone()))
            .bind(("uri", uri.clone()))
            .await;

        let Ok(mut source_embeddings) = source_embeddings else {
            error!("Failed to query source embeddings: {} {}", source, uri);
            return Ok(IndexResult::Failed);
        };

        let source_embeddings: Vec<SourceEmbeddings> = source_embeddings.take(0).map_err(SystemActorError::from)?;
        if !source_embeddings.is_empty() {
            debug!("Source already indexed with URI: {} {}", source, uri);
            return Ok(IndexResult::Cached);
        }

        // Map types if needed
        let (text_type, content) = match text_type {
            // Convert html to markdown
            TextType::Code(CodeLanguage::Html) => (TextType::Markdown, mdka::from_html(&content)),
            // PDF is already markdown
            TextType::Pdf => (TextType::Markdown, content),
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

        let start_time = std::time::Instant::now();

        let chunks = chunks.iter().map(|c| c.to_string()).collect::<Vec<String>>();
        let metadata = chunks
            .iter()
            .enumerate()
            .map(|(i, _)| ChunkMetadata {
                source: source.clone(),
                text_type: text_type.clone(),
                chunk_number: i,
                uri: uri.clone(),
            })
            .map(|metadata| serde_json::to_value(metadata).unwrap_or_default())
            .collect::<Vec<Value>>();
        let chunks_time = start_time.elapsed();
        debug!("Generated {} chunks in {:?}", chunks.len(), chunks_time);

        let start_time = std::time::Instant::now();

        let chunk_batches = chunks.chunks(chunk_batch_size);
        let metadata_batches = metadata.chunks(chunk_batch_size);
        let mut embeddings_count = 0;

        for (chunk_batch, metadata_batch) in chunk_batches.zip(metadata_batches) {
            let result = ctx
                .send::<Embeddings, StoreTextEmbeddings>(
                    StoreTextEmbeddings {
                        source: source.clone(),
                        texts: chunk_batch.to_vec(),
                        metadata: Some(metadata_batch.to_vec()),
                        tag: Some(self.tag.clone().to_string()),
                    },
                    embeddings_id,
                    SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
                )
                .await;
            match result {
                Ok(_result) => {
                    embeddings_count += 1;
                }
                Err(e) => {
                    error!("Failed to generate embeddings: {} {}", e, source);
                }
            }
        }

        let embeddings_time = start_time.elapsed();
        debug!("Generated embeddings in {:?}", embeddings_time);

        Ok(IndexResult::Indexed(embeddings_count))
    }

    async fn index_image(
        &self,
        ctx: &mut ActorContext<Self>,
        source: String,
        caption: Option<String>,
        embeddings_id: &ActorId,
    ) -> Result<IndexResult, IndexerError> {
        // Simplified query to only check source
        let source_embeddings = ctx
            .engine()
            .db()
            .query("SELECT source, ->source_image_embeddings.out AS embeddings FROM source:{source:$source, tag:$tag}")
            .bind(("source", source.clone()))
            .bind(("tag", self.tag.clone()))
            .await;

        let Ok(mut source_embeddings) = source_embeddings else {
            error!("Failed to query source embeddings: {}", source);
            return Ok(IndexResult::Failed);
        };

        let source_embeddings: Vec<SourceImageEmbeddings> =
            source_embeddings.take(0).map_err(SystemActorError::from)?;
        if !source_embeddings.is_empty() {
            debug!("Image already indexed: {}", source);
            return Ok(IndexResult::Cached);
        }

        // Extract image metadata
        let source_clone = source.clone();
        let metadata = tokio::task::spawn_blocking(move || {
            let file = std::fs::File::open(&source_clone).ok()?;
            let reader = std::io::BufReader::new(file);

            // Get image format and dimensions
            let format = image::ImageReader::new(reader).with_guessed_format().ok()?;
            let format_type = format.format();
            let dimensions = match format_type {
                Some(_) => format.into_dimensions().ok()?,
                None => return None,
            };

            // Get file metadata
            let file_metadata = std::fs::metadata(&source_clone).ok()?;

            Some(json!({
                "source": source_clone,
                "format": format_type.map(|f| f.extensions_str()[0]).unwrap_or("unknown"),
                "dimensions": {
                    "width": dimensions.0,
                    "height": dimensions.1
                },
                "size_bytes": file_metadata.len(),
                "modified": file_metadata.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs(),
                "created": file_metadata.created().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs(),
            }))
        })
        .await
        .map_err(|e| IndexerError::IO(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))?;

        let start_time = std::time::Instant::now();

        // Create the image embedding request
        let result = ctx
            .send::<Embeddings, StoreImageEmbeddings>(
                StoreImageEmbeddings {
                    source: source.clone(),
                    images: vec![Image { path: source, caption }],
                    metadata: metadata.map(|m| vec![m]),
                    tag: Some(self.tag.clone().to_string()),
                },
                embeddings_id,
                SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
            )
            .await;

        match result {
            Ok(_) => {
                let embeddings_time = start_time.elapsed();
                debug!("Generated image embedding in {:?}", embeddings_time);
                Ok(IndexResult::Indexed(1))
            }
            Err(e) => {
                error!("Failed to generate image embedding: {}", e);
                Ok(IndexResult::Failed)
            }
        }
    }
}

impl Message<IndexTexts> for Indexer {
    type Response = Indexed;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, message: &IndexTexts) -> Result<Indexed, IndexerError> {
        let Some(embeddings_id) = &self.embeddings_id else {
            return Err(IndexerError::EmbeddingsActorNotInitialized);
        };

        let total_index_texts_time = std::time::Instant::now();
        let mut indexed = 0;
        let mut cached = 0;

        for SourcedText { source, text, text_type } in message.texts.iter() {
            match self
                .index_text(
                    ctx,
                    source.clone(),
                    String::new(),
                    text.clone(),
                    text_type.clone(),
                    message.chunk_capacity.clone(),
                    message.chunk_overlap,
                    message.chunk_batch_size,
                    embeddings_id,
                )
                .await?
            {
                IndexResult::Indexed(count) => {
                    if count > 0 {
                        indexed += 1;
                    }
                }
                IndexResult::Cached => cached += 1,
                IndexResult::Failed => continue,
            }
        }

        info!("Indexed {} texts, cached {} paths, in {:?}", indexed, cached, total_index_texts_time.elapsed());
        Ok(Indexed { indexed, cached })
    }
}

impl Message<IndexGlobs> for Indexer {
    type Response = Indexed;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, message: &IndexGlobs) -> Result<Indexed, IndexerError> {
        let Some(embeddings_id) = &self.embeddings_id else {
            return Err(IndexerError::EmbeddingsActorNotInitialized);
        };
        let Some(pdf_analyzer_id) = &self.pdf_analyzer_id else {
            return Err(IndexerError::PdfAnalyzerActorNotInitialized);
        };

        let total_index_globs_time = std::time::Instant::now();
        let mut indexed = 0;
        let mut cached = 0;
        for glob in message.globs.iter() {
            let local_store_dir = ctx.engine().local_store_dir();

            // If glob is not absolute, make it relative to local_store_dir
            let full_glob = if std::path::Path::new(glob).is_absolute() {
                glob.clone()
            } else {
                local_store_dir.join(glob).to_string_lossy().into_owned()
            };

            info!("Indexing glob: {}", &full_glob);
            let task_glob = full_glob.clone();

            // Use the modified glob path in the spawn_blocking task
            let paths = tokio::task::spawn_blocking(move || {
                let mut paths = Vec::new();
                for entry in glob::glob(&task_glob).unwrap().flatten() {
                    if entry.is_file() {
                        paths.push(entry);
                    } else if entry.is_dir() {
                        // Use WalkDir to recursively collect all files
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
                warn!("Skipping glob: {}", &glob);
                continue;
            };

            for pathbuf in paths {
                info!("Indexing path: {}", &pathbuf.display());
                let ext = pathbuf.extension().and_then(|ext| ext.to_str());

                // Use the full_glob as the source instead of the original glob
                let source = full_glob.clone();
                // Use the full path as the URI
                let uri = pathbuf.to_string_lossy().to_string();

                let text_type = match ext {
                    Some("md") => TextType::Markdown,
                    Some("rs") => TextType::Code(CodeLanguage::Rust),
                    Some("py") => TextType::Code(CodeLanguage::Python),
                    Some("cue") => TextType::Code(CodeLanguage::Cue),
                    Some("html") => TextType::Code(CodeLanguage::Html),
                    Some("cpp") => TextType::Code(CodeLanguage::Cpp),
                    Some("h") => TextType::Text,
                    Some("pdf") => TextType::Pdf,
                    _ => TextType::Text,
                };

                let content = match &text_type {
                    TextType::Pdf => {
                        let result = ctx
                            .send::<PdfAnalyzer, AnalyzePdf>(
                                AnalyzePdf { file_path: pathbuf.clone() },
                                pdf_analyzer_id,
                                SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
                            )
                            .await;

                        let file_content = match result {
                            Ok(content) => content,
                            Err(e) => {
                                error!("Failed to convert pdf to md: {}. Error: {}", pathbuf.display(), e);
                                continue;
                            }
                        };

                        Ok(file_content)
                    }
                    _ => tokio::fs::read_to_string(&pathbuf).await,
                };

                let Ok(content) = content else {
                    error!("Failed to index file: {}", pathbuf.display());
                    continue;
                };

                match self
                    .index_text(
                        ctx,
                        source,
                        uri,
                        content,
                        text_type,
                        message.chunk_capacity.clone(),
                        message.chunk_overlap,
                        message.chunk_batch_size,
                        embeddings_id,
                    )
                    .await?
                {
                    IndexResult::Indexed(count) => {
                        if count > 0 {
                            indexed += 1;
                        }
                    }
                    IndexResult::Cached => cached += 1,
                    IndexResult::Failed => continue,
                }
            }
        }
        info!("Indexed {} paths, cached {} paths, in {:?}", indexed, cached, total_index_globs_time.elapsed());
        Ok(Indexed { indexed, cached })
    }
}

impl Message<DeleteSource> for Indexer {
    type Response = DeletedSource;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: &DeleteSource,
    ) -> Result<DeletedSource, IndexerError> {
        let query = include_str!("../sql/del_source.surql");
        let local_store_dir = ctx.engine().local_store_dir();
        let db = ctx.engine().db();

        let mut total_deleted = 0;
        let mut deleted_sources = Vec::new();
        let mut not_found_sources = Vec::new();

        for source in &message.sources {
            // If source is not absolute, make it relative to local_store_dir
            let full_source = if std::path::Path::new(source).is_absolute() {
                source.clone()
            } else {
                local_store_dir.join(source).to_string_lossy().into_owned()
            };

            // Delete from database
            let mut results = db
                .query(query)
                .bind(("source", full_source.clone()))
                .bind(("tag", self.tag.clone()))
                .await
                .map_err(SystemActorError::from)?;

            let deleted_count = results.take::<Option<usize>>(0).map_err(SystemActorError::from)?.unwrap_or(0);
            total_deleted += deleted_count;

            // Check if file/directory exists before attempting deletion
            let source_path = std::path::Path::new(&full_source);
            if source_path.exists() {
                // Handle both files and directories
                if source_path.is_dir() {
                    tokio::fs::remove_dir_all(source_path).await.ok();
                } else {
                    tokio::fs::remove_file(source_path).await.ok();
                }
                deleted_sources.push(source.clone());
            } else {
                not_found_sources.push(source.clone());
            }
        }

        Ok(DeletedSource { deleted_embeddings: total_deleted, deleted_sources, not_found_sources })
    }
}

impl Message<IndexImages> for Indexer {
    type Response = Indexed;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, message: &IndexImages) -> Result<Indexed, IndexerError> {
        let Some(embeddings_id) = &self.embeddings_id else {
            return Err(IndexerError::EmbeddingsActorNotInitialized);
        };

        let total_index_images_time = std::time::Instant::now();
        let mut indexed = 0;
        let mut cached = 0;
        let local_store_dir = ctx.engine().local_store_dir().to_owned();

        for image in &message.images {
            // If path is not absolute, make it relative to local_store_dir
            let source = if std::path::Path::new(&image.path).is_absolute() {
                image.path.clone()
            } else {
                local_store_dir.join(&image.path).to_string_lossy().into_owned()
            };

            info!("Indexing image: {}", &source);

            match self.index_image(ctx, source, image.caption.clone(), embeddings_id).await? {
                IndexResult::Indexed(count) => {
                    if count > 0 {
                        indexed += 1;
                    }
                }
                IndexResult::Failed => {
                    warn!("Failed to index image: {}", &image.path);
                }
                IndexResult::Cached => {
                    cached += 1;
                    debug!("Image already indexed: {}", &image.path);
                }
            }
        }

        info!("Indexed {} images, cached {} images, in {:?}", indexed, cached, total_index_images_time.elapsed());
        Ok(Indexed { indexed, cached })
    }
}

#[derive(bon::Builder, Debug, Serialize, Deserialize)]
pub struct Indexer {
    pub embeddings: Embeddings,
    pub pdf_analyzer: PdfAnalyzer,
    #[builder(default = DEFAULT_INDEXER_TAG.into())]
    pub tag: Cow<'static, str>,
    embeddings_id: Option<ActorId>,
    pdf_analyzer_id: Option<ActorId>,
}

impl Default for Indexer {
    fn default() -> Self {
        Self {
            embeddings: Embeddings::default(),
            pdf_analyzer: PdfAnalyzer::default(),
            tag: DEFAULT_INDEXER_TAG.into(),
            embeddings_id: None,
            pdf_analyzer_id: None,
        }
    }
}

impl Actor for Indexer {
    type Error = IndexerError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), IndexerError> {
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

        info!("Indexer ready");

        // Start the message stream
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(input) = frame.is::<IndexGlobs>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            } else if let Some(input) = frame.is::<DeleteSource>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            } else if let Some(input) = frame.is::<IndexImages>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            }
        }

        embeddings_handle.abort();
        pdf_analyzer_handle.abort();
        Ok(())
    }
}
