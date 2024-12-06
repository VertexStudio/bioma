BEGIN;

-- Get all sources matching the regex pattern
LET $matching_sources = SELECT * FROM source WHERE string::matches(id.uri, $source);

-- Get all embeddings connected to matching sources 
LET $embeddings = SELECT ->NomicEmbedTextV15_source_embeddings.out AS embedding 
FROM $matching_sources;

LET $flat_embeddings = array::flatten($embeddings.embedding);

-- Delete all related records
DELETE NomicEmbedTextV15_model_embeddings WHERE out IN $flat_embeddings;
DELETE NomicEmbedTextV15_source_embeddings WHERE in IN $matching_sources;
DELETE NomicEmbedTextV15_embedding WHERE id IN $flat_embeddings;

-- Delete sources and capture their paths with structured information
LET $deleted_sources = SELECT VALUE id.{source, uri} FROM (DELETE $matching_sources RETURN BEFORE);

RETURN {
    deleted_embeddings: count($flat_embeddings),
    deleted_sources: $deleted_sources
};

COMMIT;