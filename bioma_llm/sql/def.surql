-- Define the model table
DEFINE TABLE model TYPE NORMAL SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD name ON model TYPE string PERMISSIONS FULL;

-- Define the embedding table
DEFINE TABLE embedding TYPE NORMAL SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD text ON embedding TYPE string PERMISSIONS FULL;
DEFINE FIELD embedding ON embedding TYPE array<number> PERMISSIONS FULL;
DEFINE INDEX embedding_index ON TABLE embedding FIELDS embedding MTREE DIMENSION 768 DIST EUCLIDEAN;

-- Define the model_embeddings table
DEFINE TABLE model_embeddings TYPE RELATION IN model OUT embedding SCHEMALESS PERMISSIONS NONE;
DEFINE FIELD in ON model_embeddings TYPE record<model> PERMISSIONS FULL;
DEFINE FIELD out ON model_embeddings TYPE record<embedding> PERMISSIONS FULL;
