use crate::embeddings::{Embeddings, EmbeddingsError, StoreTextEmbeddings};
use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use std::path::PathBuf;
use surrealdb::RecordId;
use text_splitter::{ChunkConfig, CodeSplitter, MarkdownSplitter, TextSplitter};
use tracing::{debug, error, info, warn};
use url::Url;

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
    #[error("Embeddings actor not found")]
    EmbeddingsActorNotFound,
}

impl ActorError for IndexerError {}

#[derive(bon::Builder, Debug, Clone, Serialize, Deserialize)]
pub struct IndexGlobs {
    pub globs: Vec<String>,
    #[builder(default = DEFAULT_CHUNK_CAPACITY)]
    #[serde(default = "default_chunk_capacity")]
    pub chunk_capacity: std::ops::Range<usize>,
    #[builder(default = DEFAULT_CHUNK_OVERLAP)]
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap: usize,
    #[builder(default = DEFAULT_CHUNK_BATCH_SIZE)]
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

impl Default for IndexGlobs {
    fn default() -> Self {
        Self {
            globs: vec![],
            chunk_capacity: DEFAULT_CHUNK_CAPACITY,
            chunk_overlap: DEFAULT_CHUNK_OVERLAP,
            chunk_batch_size: DEFAULT_CHUNK_BATCH_SIZE,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedGlobs {
    pub indexed: usize,
    pub cached: usize,
}

pub enum CodeLanguage {
    Rust,
    Python,
    Cue,
    Cpp,
    Html,
}

pub enum TextType {
    Markdown,
    Code(CodeLanguage),
    Text,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Source {
    File(PathBuf),
    Url(Url),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkMetadata {
    pub source: Source,
    pub chunk_number: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SourceEmbeddings {
    pub source: String,
    pub embeddings: Vec<RecordId>,
}

impl Message<IndexGlobs> for Indexer {
    type Response = IndexedGlobs;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: &IndexGlobs,
    ) -> Result<IndexedGlobs, IndexerError> {
        let Some(embeddings_id) = &self.embeddings_id else {
            return Err(IndexerError::EmbeddingsActorNotFound);
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

                let path = pathbuf.to_string_lossy().to_string();

                let source_embeddings = ctx
                    .engine()
                    .db()
                    .query(
                        "SELECT source, ->source_embeddings.out as embeddings FROM source:{source:$source, tag:$tag}",
                    )
                    .bind(("source", path.clone()))
                    .bind(("tag", self.tag.clone()))
                    .await;
                let Ok(mut source_embeddings) = source_embeddings else {
                    error!("Failed to query source embeddings: {}", path);
                    continue;
                };
                let source_embeddings: Vec<SourceEmbeddings> =
                    source_embeddings.take(0).map_err(SystemActorError::from)?;
                if source_embeddings.len() > 0 {
                    debug!("Path already indexed: {}", path);
                    cached += 1;
                    continue;
                }

                let ext = pathbuf.extension().and_then(|ext| ext.to_str());
                let text_type = match ext {
                    Some("md") => TextType::Markdown,
                    Some("rs") => TextType::Code(CodeLanguage::Rust),
                    Some("py") => TextType::Code(CodeLanguage::Python),
                    Some("cue") => TextType::Code(CodeLanguage::Cue),
                    Some("html") => TextType::Code(CodeLanguage::Html),
                    Some("cpp") => TextType::Code(CodeLanguage::Cpp),
                    Some("h") => TextType::Text,
                    _ => TextType::Text,
                };

                info!("Indexing path: {}", &pathbuf.display());
                let content = tokio::fs::read_to_string(&pathbuf).await;
                let Ok(content) = content else {
                    error!("Failed to index file: {}", pathbuf.display());
                    continue;
                };

                // Convert html to markdown
                let (text_type, content) = match text_type {
                    TextType::Code(CodeLanguage::Html) => (TextType::Markdown, mdka::from_html(&content)),
                    _ => (text_type, content),
                };

                let chunks = match text_type {
                    TextType::Text => {
                        let splitter = TextSplitter::new(
                            ChunkConfig::new(message.chunk_capacity.clone())
                                .with_trim(false)
                                .with_overlap(message.chunk_overlap)?,
                        );
                        splitter.chunks(&content).collect::<Vec<&str>>()
                    }
                    TextType::Markdown => {
                        let splitter = MarkdownSplitter::new(
                            ChunkConfig::new(message.chunk_capacity.clone())
                                .with_trim(false)
                                .with_overlap(message.chunk_overlap)?,
                        );
                        splitter.chunks(&content).collect::<Vec<&str>>()
                    }
                    TextType::Code(CodeLanguage::Rust) => {
                        let splitter = CodeSplitter::new(
                            tree_sitter_rust::LANGUAGE,
                            ChunkConfig::new(message.chunk_capacity.clone())
                                .with_trim(false)
                                .with_overlap(message.chunk_overlap)?,
                        )
                        .expect("Invalid tree-sitter language");
                        splitter.chunks(&content).collect::<Vec<&str>>()
                    }
                    TextType::Code(CodeLanguage::Python) => {
                        let splitter = CodeSplitter::new(
                            tree_sitter_python::LANGUAGE,
                            ChunkConfig::new(message.chunk_capacity.clone())
                                .with_trim(false)
                                .with_overlap(message.chunk_overlap)?,
                        )
                        .expect("Invalid tree-sitter language");
                        splitter.chunks(&content).collect::<Vec<&str>>()
                    }
                    TextType::Code(CodeLanguage::Cue) => {
                        let splitter = CodeSplitter::new(
                            tree_sitter_cue::LANGUAGE,
                            ChunkConfig::new(message.chunk_capacity.clone())
                                .with_trim(false)
                                .with_overlap(message.chunk_overlap)?,
                        )
                        .expect("Invalid tree-sitter language");
                        splitter.chunks(&content).collect::<Vec<&str>>()
                    }
                    TextType::Code(CodeLanguage::Cpp) => {
                        let splitter = CodeSplitter::new(
                            tree_sitter_cpp::LANGUAGE,
                            ChunkConfig::new(message.chunk_capacity.clone())
                                .with_trim(false)
                                .with_overlap(message.chunk_overlap)?,
                        )
                        .expect("Invalid tree-sitter language");
                        splitter.chunks(&content).collect::<Vec<&str>>()
                    }
                    TextType::Code(CodeLanguage::Html) => {
                        panic!("Should have been converted to markdown");
                    }
                };

                let start_time = std::time::Instant::now();

                let chunks = chunks.iter().map(|c| c.to_string()).collect::<Vec<String>>();
                let metadata = chunks
                    .iter()
                    .enumerate()
                    .map(|(i, _)| ChunkMetadata { source: Source::File(pathbuf.clone()), chunk_number: i })
                    .map(|metadata| serde_json::to_value(metadata).unwrap_or_default())
                    .collect::<Vec<Value>>();
                let chunks_time = start_time.elapsed();
                debug!("Generated {} chunks in {:?}", chunks.len(), chunks_time);

                let start_time = std::time::Instant::now();

                let chunk_batches = chunks.chunks(message.chunk_batch_size);
                let metadata_batches = metadata.chunks(message.chunk_batch_size);
                let mut embeddings_count = 0;
                for (chunk_batch, metadata_batch) in chunk_batches.zip(metadata_batches) {
                    let result = ctx
                        .send::<Embeddings, StoreTextEmbeddings>(
                            StoreTextEmbeddings {
                                source: path.clone(),
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
                            self.save(ctx).await?;
                        }
                        Err(e) => {
                            error!("Failed to generate embeddings: {} {}", e, path);
                        }
                    }
                }

                if embeddings_count > 0 {
                    indexed += 1;
                }
                let embeddings_time = start_time.elapsed();
                debug!("Generated embeddings in {:?}", embeddings_time);
            }
        }
        info!("Indexed {} paths, cached {} paths, in {:?}", indexed, cached, total_index_globs_time.elapsed());
        Ok(IndexedGlobs { indexed, cached })
    }
}

#[derive(bon::Builder, Debug, Serialize, Deserialize)]
pub struct Indexer {
    pub embeddings: Embeddings,
    #[builder(default = DEFAULT_INDEXER_TAG.into())]
    pub tag: Cow<'static, str>,
    embeddings_id: Option<ActorId>,
}

impl Default for Indexer {
    fn default() -> Self {
        Self { embeddings: Embeddings::default(), tag: DEFAULT_INDEXER_TAG.into(), embeddings_id: None }
    }
}

impl Actor for Indexer {
    type Error = IndexerError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), IndexerError> {
        let self_id = ctx.id().clone();
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
        Ok(())
    }
}
