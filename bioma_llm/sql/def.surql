-- Define the model table
DEFINE TABLE model TYPE NORMAL SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD name ON model TYPE string PERMISSIONS FULL;

-- Define the embedding table for {prefix}
DEFINE TABLE {prefix}_embedding TYPE NORMAL SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD text ON {prefix}_embedding TYPE option<string> PERMISSIONS FULL;
DEFINE FIELD embedding ON {prefix}_embedding TYPE array<number> PERMISSIONS FULL;
DEFINE INDEX {prefix}_embedding_index ON TABLE {prefix}_embedding FIELDS embedding MTREE DIMENSION {dim} DIST EUCLIDEAN;

-- Add Full-Text Search index for the embedding text field
DEFINE ANALYZER custom_analyzer TOKENIZERS blank FILTERS lowercase, snowball(english);
DEFINE INDEX {prefix}_embedding_text_search ON {prefix}_embedding FIELDS text SEARCH ANALYZER custom_analyzer BM25 HIGHLIGHTS;

-- Define the source table
DEFINE TABLE source TYPE NORMAL SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD source ON source TYPE string PERMISSIONS FULL;

-- Define the source_embeddings table
DEFINE TABLE {prefix}_source_embeddings TYPE RELATION IN source OUT {prefix}_embedding SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD in ON {prefix}_source_embeddings TYPE record<source> PERMISSIONS FULL;
DEFINE FIELD out ON {prefix}_source_embeddings TYPE record<{prefix}_embedding> PERMISSIONS FULL;

-- Define the model_embeddings table
DEFINE TABLE {prefix}_model_embeddings TYPE RELATION IN model OUT {prefix}_embedding SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD in ON {prefix}_model_embeddings TYPE record<model> PERMISSIONS FULL;
DEFINE FIELD out ON {prefix}_model_embeddings TYPE record<{prefix}_embedding> PERMISSIONS FULL;