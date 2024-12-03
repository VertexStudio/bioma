BEGIN;

-- Get all sources matching the regex pattern
LET $matching_sources = SELECT * FROM source WHERE string::matches(source, $source);

-- Get all embeddings connected to matching sources 
LET $image_embeddings = SELECT ->{prefix}_source_embeddings.out as embedding 
FROM $matching_sources;

LET $flat_image_embeddings = array::flatten($image_embeddings.embedding);

-- Delete all related records
DELETE {prefix}_model_embeddings WHERE out IN $flat_image_embeddings;
DELETE {prefix}_source_embeddings WHERE in IN $matching_sources;
DELETE {prefix}_embedding WHERE id IN $flat_image_embeddings;

-- Delete sources and capture their paths
LET $deleted_sources = (DELETE source WHERE string::matches(source, $source) RETURN BEFORE).source;

RETURN {
    total_deleted: count($flat_image_embeddings),
    deleted_sources: $deleted_sources
};

COMMIT;