use bioma_llm::prelude::IndexGlobs;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// /index Endpoint Schemas

#[derive(ToSchema, Clone, Serialize, Deserialize)]
pub struct IndexGlobsRequest {
    pub globs: Vec<String>,
    pub chunk_capacity: ChunkCapacity,
    pub chunk_overlap: usize,
    pub chunk_batch_size: usize,
}

#[derive(ToSchema, Clone, Serialize, Deserialize)]
pub struct ChunkCapacity {
    pub start: usize,
    pub end: usize,
}

impl Into<IndexGlobs> for IndexGlobsRequest {
    fn into(self) -> IndexGlobs {
        IndexGlobs {
            globs: self.globs,
            chunk_capacity: self.chunk_capacity.start..self.chunk_capacity.end,
            chunk_overlap: self.chunk_overlap,
            chunk_batch_size: self.chunk_batch_size,
        }
    }
}
