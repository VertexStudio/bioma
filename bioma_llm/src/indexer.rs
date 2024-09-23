use crate::embeddings::{Embeddings, EmbeddingsError, GenerateEmbeddings};
use bioma_actor::prelude::*;
use bloomfilter::Bloom;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use text_splitter::{ChunkConfig, CodeSplitter, MarkdownSplitter, TextSplitter};
use tracing::{debug, error, info, warn};
use url::Url;

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
}

impl ActorError for IndexerError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexGlobs {
    pub globs: Vec<String>,
    pub chunk_capacity: std::ops::Range<usize>,
    pub chunk_overlap: usize,
}

impl Default for IndexGlobs {
    fn default() -> Self {
        Self { globs: vec![], chunk_capacity: 500..2000, chunk_overlap: 200 }
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

impl Message<IndexGlobs> for Indexer {
    type Response = IndexedGlobs;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: &IndexGlobs,
    ) -> Result<IndexedGlobs, IndexerError> {
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
            for path in paths {
                let Ok(path) = path else {
                    warn!("Skipping path: {:?}", path);
                    continue;
                };
                if self.indexed.check(&path) {
                    debug!("Path already indexed: {}", path.display());
                    cached += 1;
                    continue;
                }
                self.indexed.set(&path);

                let ext = path.extension().and_then(|ext| ext.to_str());
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

                info!("Indexing path: {}", &path.display());
                let content = tokio::fs::read_to_string(&path).await;
                let Ok(content) = content else {
                    error!("Failed to index file: {}", path.display());
                    continue;
                };

                // Convert html to markdown
                let content = match text_type {
                    TextType::Code(CodeLanguage::Html) => mdka::from_html(&content),
                    _ => content,
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
                let file_name = path.to_string_lossy().to_string();
                let chunks = chunks.iter().map(|c| c.to_string()).collect::<Vec<String>>();
                let metadata = chunks
                    .iter()
                    .enumerate()
                    .map(|(i, _)| ChunkMetadata { source: Source::File(path.clone()), chunk_number: i })
                    .map(|metadata| serde_json::to_value(metadata).unwrap_or_default())
                    .collect::<Vec<Value>>();
                let chunks_time = start_time.elapsed();
                debug!("Generated {} chunks in {:?}", chunks.len(), chunks_time);

                let start_time = std::time::Instant::now();
                let result = ctx
                    .send::<Embeddings, GenerateEmbeddings>(
                        GenerateEmbeddings {
                            texts: chunks,
                            metadata: Some(metadata),
                            tag: Some("indexer_content".to_string()),
                        },
                        &self.embeddings_actor,
                        SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
                    )
                    .await;
                let embeddings_time = start_time.elapsed();
                debug!("Generated embeddings in {:?}", embeddings_time);

                match result {
                    Ok(_result) => {
                        indexed += 1;
                        self.save(ctx).await?;
                    }
                    Err(e) => {
                        error!("Failed to generate embeddings: {} {}", e, file_name);
                    }
                }
            }
        }
        info!("Indexed {} paths, cached {} paths, in {:?}", indexed, cached, total_index_globs_time.elapsed());
        Ok(IndexedGlobs { indexed, cached })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Indexer {
    pub embeddings_actor: ActorId,
    pub indexed: Bloom<PathBuf>,
}

impl Default for Indexer {
    fn default() -> Self {
        let false_positive_rate = 0.01;
        let num_elements = 100_000;
        let bloom = Bloom::new_for_fp_rate(num_elements, false_positive_rate);
        let embeddings_actor_id = ActorId::of::<Embeddings>("/indexer/embeddings");
        Self { embeddings_actor: embeddings_actor_id.clone(), indexed: bloom }
    }
}

impl Actor for Indexer {
    type Error = IndexerError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), IndexerError> {
        let (mut embeddings_ctx, mut embeddings_actor) = Actor::spawn(
            ctx.engine().clone(),
            self.embeddings_actor.clone(),
            Embeddings { model_name: "nomic-embed-text".to_string(), ..Default::default() },
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;
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
