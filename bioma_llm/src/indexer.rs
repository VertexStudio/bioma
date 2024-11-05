use crate::{
    embeddings::{Embeddings, EmbeddingsError, StoreTextEmbeddings},
    pdf_analyzer::{AnalyzePdf, PdfAnalyzer, PdfAnalyzerError},
};
use bioma_actor::prelude::*;
use derive_more::Display;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use surrealdb::RecordId;
use text_splitter::{ChunkConfig, CodeSplitter, MarkdownSplitter, TextSplitter};
use tracing::{debug, error, info, warn};

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SourceEmbeddings {
    pub source: String,
    pub embeddings: Vec<RecordId>,
}

#[derive(Debug)]
enum IndexResult {
    Indexed(usize),
    Cached,
    Failed,
}

impl Indexer {
    async fn index_text(
        &self,
        ctx: &mut ActorContext<Self>,
        source: String,
        content: String,
        text_type: TextType,
        chunk_capacity: std::ops::Range<usize>,
        chunk_overlap: usize,
        chunk_batch_size: usize,
        embeddings_id: &ActorId,
    ) -> Result<IndexResult, IndexerError> {
        // Check if source is already indexed
        let source_embeddings = ctx
            .engine()
            .db()
            .query("SELECT source, ->source_embeddings.out as embeddings FROM source:{source:$source, tag:$tag}")
            .bind(("source", source.clone()))
            .bind(("tag", self.tag.clone()))
            .await;
        let Ok(mut source_embeddings) = source_embeddings else {
            error!("Failed to query source embeddings: {}", source);
            return Ok(IndexResult::Failed);
        };
        let source_embeddings: Vec<SourceEmbeddings> = source_embeddings.take(0).map_err(SystemActorError::from)?;
        if source_embeddings.len() > 0 {
            debug!("Source already indexed: {}", source);
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
            .map(|(i, _)| ChunkMetadata { source: source.clone(), text_type: text_type.clone(), chunk_number: i })
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
            info!("Indexing glob: {}", &glob);
            let task_glob = glob.clone();
            let paths = tokio::task::spawn_blocking(move || glob::glob(&task_glob)).await;
            let Ok(Ok(paths)) = paths else {
                warn!("Skipping glob: {}", &glob);
                continue;
            };
            for pathbuf in paths {
                let Ok(pathbuf) = pathbuf else {
                    warn!("Skipping path: {:?}", pathbuf);
                    continue;
                };

                info!("Indexing path: {}", &pathbuf.display());
                let ext = pathbuf.extension().and_then(|ext| ext.to_str());

                info!("ext: {:?}", ext);
                let source = pathbuf.to_string_lossy().to_string();

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
                                AnalyzePdf { file_path: source.clone().into() },
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
            }
        }

        embeddings_handle.abort();
        pdf_analyzer_handle.abort();
        Ok(())
    }
}
