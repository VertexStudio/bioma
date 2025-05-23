-- Define the model table
DEFINE TABLE model TYPE NORMAL SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD name ON model TYPE string PERMISSIONS FULL;

-- Define the embedding table for {prefix}
DEFINE TABLE {prefix}_embedding TYPE NORMAL SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD text ON {prefix}_embedding TYPE option<string> PERMISSIONS FULL;
DEFINE FIELD embedding ON {prefix}_embedding TYPE array<number> PERMISSIONS FULL;

-- Add Full-Text Search index for the embedding text field
DEFINE ANALYZER custom_analyzer TOKENIZERS blank FILTERS lowercase, snowball(english);
DEFINE INDEX {prefix}_embedding_text_search ON {prefix}_embedding FIELDS text SEARCH ANALYZER custom_analyzer BM25 HIGHLIGHTS;

-- Define the source table
DEFINE TABLE source TYPE NORMAL SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD summary ON source TYPE option<string> PERMISSIONS FULL;
DEFINE INDEX source_idx ON source FIELDS id.source;
-- DEFINE FIELD source ON source TYPE string PERMISSIONS FULL;
-- DEFINE FIELD uri ON source TYPE string PERMISSIONS FULL;
-- DEFINE INDEX source_uri_unique_idx ON source FIELDS source, uri UNIQUE;

-- Define the source_embeddings table
DEFINE TABLE {prefix}_source_embeddings TYPE RELATION IN source OUT {prefix}_embedding SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD in ON {prefix}_source_embeddings TYPE record<source> PERMISSIONS FULL;
DEFINE FIELD out ON {prefix}_source_embeddings TYPE record<{prefix}_embedding> PERMISSIONS FULL;
-- Optimizes filtering by source in relation queries
-- Critical for performant source-based filtering when querying directly from the relation table
-- Used heavily in similarity searches that need to scope results to specific sources
DEFINE INDEX {prefix}_source_embeddings_in_id_source_index ON {prefix}_source_embeddings FIELDS in.id.source;
-- Optimizes vector similarity searches directly on the relation table
-- Allows efficient vector similarity operations without having to join with the embedding table
-- Enables querying embeddings by similarity while filtering by source in a single operation
DEFINE INDEX {prefix}_source_embeddings_out_embedding_index ON {prefix}_source_embeddings FIELDS out.embedding MTREE DIMENSION {dim} DIST COSINE TYPE F32 CAPACITY 40 DOC_IDS_ORDER 1000 DOC_IDS_CACHE 2000 MTREE_CACHE 2000;

-- Define the model_embeddings table
DEFINE TABLE {prefix}_model_embeddings TYPE RELATION IN model OUT {prefix}_embedding SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD in ON {prefix}_model_embeddings TYPE record<model> PERMISSIONS FULL;
DEFINE FIELD out ON {prefix}_model_embeddings TYPE record<{prefix}_embedding> PERMISSIONS FULL;
-- DEFINE INDEX {prefix}_model_embeddings_in_idx ON TABLE {prefix}_model_embeddings FIELDS in;