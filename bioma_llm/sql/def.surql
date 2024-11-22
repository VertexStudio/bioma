-- Define the model table
DEFINE TABLE model TYPE NORMAL SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD name ON model TYPE string PERMISSIONS FULL;

-- Define the text_embedding table
DEFINE TABLE text_embedding TYPE NORMAL SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD text ON text_embedding TYPE string PERMISSIONS FULL;
DEFINE FIELD embedding ON text_embedding TYPE array<number> PERMISSIONS FULL;
DEFINE INDEX text_embedding_index ON TABLE text_embedding FIELDS embedding MTREE DIMENSION 768 DIST EUCLIDEAN;

-- Define the image_embedding table
DEFINE TABLE image_embedding TYPE NORMAL SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD caption ON image_embedding TYPE string PERMISSIONS FULL;
DEFINE FIELD embedding ON image_embedding TYPE array<number> PERMISSIONS FULL;
DEFINE INDEX image_embedding_index ON TABLE image_embedding FIELDS embedding MTREE DIMENSION 512 DIST EUCLIDEAN;

-- Add Full-Text Search indices
DEFINE ANALYZER custom_analyzer TOKENIZERS blank FILTERS lowercase, snowball(english);
DEFINE INDEX text_embedding_text_search ON text_embedding FIELDS text SEARCH ANALYZER custom_analyzer BM25 HIGHLIGHTS;
DEFINE INDEX image_embedding_caption_search ON image_embedding FIELDS caption SEARCH ANALYZER custom_analyzer BM25 HIGHLIGHTS;

-- Define the source table
DEFINE TABLE source TYPE NORMAL SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD source ON source TYPE string PERMISSIONS FULL;

-- Define the source_text_embeddings table
DEFINE TABLE source_text_embeddings TYPE RELATION IN source OUT text_embedding SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD in ON source_text_embeddings TYPE record<source> PERMISSIONS FULL;
DEFINE FIELD out ON source_text_embeddings TYPE record<text_embedding> PERMISSIONS FULL;

-- Define the source_image_embeddings table
DEFINE TABLE source_image_embeddings TYPE RELATION IN source OUT image_embedding SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD in ON source_image_embeddings TYPE record<source> PERMISSIONS FULL;
DEFINE FIELD out ON source_image_embeddings TYPE record<image_embedding> PERMISSIONS FULL;

-- Define the model_text_embeddings table
DEFINE TABLE model_text_embeddings TYPE RELATION IN model OUT text_embedding SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD in ON model_text_embeddings TYPE record<model> PERMISSIONS FULL;
DEFINE FIELD out ON model_text_embeddings TYPE record<text_embedding> PERMISSIONS FULL;

-- Define the model_image_embeddings table
DEFINE TABLE model_image_embeddings TYPE RELATION IN model OUT image_embedding SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD in ON model_image_embeddings TYPE record<model> PERMISSIONS FULL;
DEFINE FIELD out ON model_image_embeddings TYPE record<image_embedding> PERMISSIONS FULL;
