use crate::{
    embeddings::{Embeddings, EmbeddingsError, ImageData, StoreEmbeddings},
    markitdown::{AnalyzeMCFile, MarkitDown, MarkitDownError},
    pdf_analyzer::{AnalyzePdf, PdfAnalyzer, PdfAnalyzerError},
};
use bioma_actor::prelude::*;
use derive_more::Display;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use surrealdb::RecordId;
use text_splitter::{ChunkConfig, CodeSplitter, MarkdownSplitter, TextSplitter};
use tracing::{error, info, warn};
use walkdir::WalkDir;

use crate::embeddings::EmbeddingContent;

const DEFAULT_CHUNK_CAPACITY: std::ops::Range<usize> = 500..2000;
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
}

impl ActorError for IndexerError {}

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
    Indexed(Vec<RecordId>),
    Failed,
}

/// The source of the embeddings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentSource {
    pub source: String,
    pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteSource {
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    Image { path: String },
}

impl Indexer {
    async fn index_content(
        &self,
        ctx: &mut ActorContext<Self>,
        source: ContentSource,
        content: Content,
        embeddings_id: &ActorId,
    ) -> Result<IndexResult, IndexerError> {
        match content {
            Content::Image { path } => {
                let path_clone = path.clone();
                // let source_clone = source.clone();
                let metadata = tokio::task::spawn_blocking(move || {
                    let file = std::fs::File::open(&path).ok()?;
                    let reader = std::io::BufReader::new(file);

                    let format = image::ImageReader::new(reader).with_guessed_format().ok()?;
                    let format_type = format.format();
                    let dimensions = match format_type {
                        Some(_) => format.into_dimensions().ok()?,
                        None => return None,
                    };

                    let file_metadata = std::fs::metadata(&path).ok()?;

                    let image_metadata = Metadata::Image(ImageMetadata {
                        format: format_type.map(|f| f.extensions_str()[0]).unwrap_or("unknown").to_string(),
                        dimensions: ImageDimensions { width: dimensions.0, height: dimensions.1 },
                        size_bytes: file_metadata.len(),
                        modified: file_metadata.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs(),
                        created: file_metadata.created().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs(),
                    });

                    Some(serde_json::to_value(image_metadata).ok()?)
                })
                .await
                .map_err(|e| IndexerError::IO(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))?;

                let result = ctx
                    .send_and_wait_reply::<Embeddings, StoreEmbeddings>(
                        StoreEmbeddings {
                            content: EmbeddingContent::Image(vec![ImageData::Path(path_clone)]),
                            metadata: metadata.map(|m| vec![m]),
                        },
                        embeddings_id,
                        SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
                    )
                    .await;

                let mut embeddings_ids: Vec<RecordId> = Vec::new();

                match result {
                    Ok(stored_embeddings) => {
                        embeddings_ids.extend(stored_embeddings.ids);
                        Ok(IndexResult::Indexed(embeddings_ids))
                    }
                    Err(e) => {
                        error!("Failed to generate image embedding: {}", e);
                        Ok(IndexResult::Failed)
                    }
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
                let mut embeddings_ids: Vec<RecordId> = Vec::new();

                for (chunk_batch, metadata_batch) in chunk_batches.zip(metadata_batches) {
                    let result = ctx
                        .send_and_wait_reply::<Embeddings, StoreEmbeddings>(
                            StoreEmbeddings {
                                content: EmbeddingContent::Text(chunk_batch.to_vec()),
                                metadata: Some(metadata_batch.to_vec()),
                            },
                            embeddings_id,
                            SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
                        )
                        .await;

                    match result {
                        Ok(stored_embeddings) => {
                            embeddings_ids.extend(stored_embeddings.ids);
                        }
                        Err(e) => error!("Failed to generate embeddings: {} {}", e, source.source),
                    }
                }

                Ok(IndexResult::Indexed(embeddings_ids))
            }
        }
    }
}

impl Message<IndexGlobs> for Indexer {
    type Response = Indexed;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, message: &IndexGlobs) -> Result<(), IndexerError> {
        let Some(embeddings_id) = &self.embeddings_id else {
            return Err(IndexerError::EmbeddingsActorNotInitialized);
        };
        let Some(pdf_analyzer_id) = &self.pdf_analyzer_id else {
            return Err(IndexerError::PdfAnalyzerActorNotInitialized);
        };
        let Some(markitdown_id) = &self.markitdown_id else {
            return Err(IndexerError::MarkitdownActorNotInitialized);
        };

        let total_index_globs_time = std::time::Instant::now();
        let mut indexed = 0;
        let mut cached = 0;

        for glob in message.globs.iter() {
            let local_store_dir = ctx.engine().local_store_dir();
            let full_glob = if std::path::Path::new(glob).is_absolute() {
                glob.clone()
            } else {
                local_store_dir.join(glob).to_string_lossy().into_owned()
            };

            info!("Indexing glob: {}", &full_glob);
            let full_glob_clone = full_glob.clone();
            let paths = tokio::task::spawn_blocking(move || {
                let mut paths = Vec::new();
                for entry in glob::glob(&full_glob).unwrap().flatten() {
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
                warn!("Skipping glob: {}", &glob);
                continue;
            };

            for pathbuf in paths {
                let uri = pathbuf.to_string_lossy().to_string();
                let source = ContentSource { source: full_glob_clone.clone(), uri: uri.clone() };

                // Check if source already exists before processing
                let query =
                    format!("SELECT id.source AS source, id.uri AS uri FROM source:{{source: $source, uri: $uri}}");
                let sources = ctx
                    .engine()
                    .db()
                    .lock()
                    .await
                    .query(&query)
                    .bind(("source", source.source.clone()))
                    .bind(("uri", source.uri.clone()))
                    .await;

                if let Ok(mut sources) = sources {
                    let sources: Vec<ContentSource> = sources.take(0).map_err(SystemActorError::from)?;
                    if !sources.is_empty() {
                        info!("Content already indexed with URI: {} {}", source.source, source.uri);
                        cached += 1;
                        continue;
                    }
                }

                info!("Indexing path: {}", &pathbuf.display());
                let ext = pathbuf.extension().and_then(|ext| ext.to_str());

                let content = if let Some(ext) = ext {
                    if IMAGE_EXTENSIONS.iter().any(|&img_ext| img_ext.eq_ignore_ascii_case(ext)) {
                        Content::Image { path: uri.clone() }
                    } else {
                        let chunk_config =
                            (message.chunk_capacity.clone(), message.chunk_overlap, message.chunk_batch_size);

                        // Special handling for PDF
                        if ext == "pdf" {
                            match ctx
                                .send_and_wait_reply::<PdfAnalyzer, AnalyzePdf>(
                                    AnalyzePdf { file_path: pathbuf.clone() },
                                    pdf_analyzer_id,
                                    SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
                                )
                                .await
                            {
                                Ok(content) => Content::Text { content, text_type: TextType::Pdf, chunk_config },
                                Err(e) => {
                                    error!("Failed to convert pdf to md: {}. Error: {}", pathbuf.display(), e);
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
                                Ok(content) => Content::Text { content, text_type: TextType::MCFile, chunk_config },
                                Err(e) => {
                                    error!("Failed to convert MC file to md: {}. Error: {}", pathbuf.display(), e);
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
                        (message.chunk_capacity.clone(), message.chunk_overlap, message.chunk_batch_size);
                    match tokio::fs::read_to_string(&pathbuf).await {
                        Ok(content) => Content::Text { content, text_type: TextType::Text, chunk_config },
                        Err(_) => continue,
                    }
                };

                let mut embeddings_ids: Vec<RecordId> = Vec::new();

                match self.index_content(ctx, source.clone(), content, embeddings_id).await? {
                    IndexResult::Indexed(ids) => {
                        embeddings_ids.extend(ids);
                        if !embeddings_ids.is_empty() {
                            indexed += 1;
                        }
                    }
                    IndexResult::Failed => continue,
                }

                if !embeddings_ids.is_empty() {
                    let source_query = include_str!("../sql/source.surql");
                    ctx.engine()
                        .db()
                        .lock()
                        .await
                        .query(*&source_query)
                        .bind(("source", source.source.clone()))
                        .bind(("uri", source.uri.clone()))
                        .bind(("emb_ids", embeddings_ids))
                        .bind(("prefix", self.embeddings.table_prefix()))
                        .await
                        .map_err(SystemActorError::from)
                        .unwrap();
                }
            }
        }

        info!("Indexed {} paths, cached {} paths, in {:?}", indexed, cached, total_index_globs_time.elapsed());
        ctx.reply(Indexed { indexed, cached }).await?;
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
            .bind(("source", message.source.clone()))
            .await
            .map_err(SystemActorError::from)?;

        let delete_result: DeletedSource = results
            .take::<Vec<DeletedSource>>(0)
            .map_err(IndexerError::from)?
            .pop()
            .ok_or(IndexerError::Other("No delete result found".to_string()))?;

        // Process file deletions
        for source in &delete_result.deleted_sources {
            let source_path = std::path::Path::new(&source.uri);
            if source_path.exists() {
                if source_path.is_dir() {
                    tokio::fs::remove_dir_all(source_path).await.ok();
                } else {
                    tokio::fs::remove_file(source_path).await.ok();
                }
            }
        }

        ctx.reply(delete_result).await?;
        Ok(())
    }
}

#[derive(bon::Builder, Debug, Serialize, Deserialize)]
pub struct Indexer {
    pub embeddings: Embeddings,
    pub pdf_analyzer: PdfAnalyzer,
    pub markitdown: MarkitDown,
    embeddings_id: Option<ActorId>,
    pdf_analyzer_id: Option<ActorId>,
    markitdown_id: Option<ActorId>,
    #[serde(skip)]
    pdf_analyzer_handle: Option<tokio::task::JoinHandle<()>>,
    #[serde(skip)]
    embeddings_handle: Option<tokio::task::JoinHandle<()>>,
    #[serde(skip)]
    markitdown_handle: Option<tokio::task::JoinHandle<()>>,
}

impl Default for Indexer {
    fn default() -> Self {
        Self {
            embeddings: Embeddings::default(),
            pdf_analyzer: PdfAnalyzer::default(),
            markitdown: MarkitDown::default(),
            embeddings_id: None,
            pdf_analyzer_id: None,
            markitdown_id: None,
            pdf_analyzer_handle: None,
            embeddings_handle: None,
            markitdown_handle: None,
        }
    }
}

impl Actor for Indexer {
    type Error = IndexerError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), IndexerError> {
        self.init(ctx).await?;

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

        self.pdf_analyzer_handle = Some(pdf_analyzer_handle);
        self.embeddings_handle = Some(embeddings_handle);
        self.markitdown_handle = Some(markitdown_handle);

        info!("Indexer ready");

        Ok(())
    }
}
