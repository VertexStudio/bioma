BEGIN;

// First define the source record
LET $source = source:{ source: $source, tag: $tag };

// Get embeddings connected to this source
LET $embeddings = SELECT ->source_embeddings.out as embedding FROM $source;

// Flatten the nested array structure
LET $flat_embeddings = array::flatten($embeddings.embedding);

// Delete in correct order to maintain referential integrity
DELETE model_embeddings WHERE out IN $flat_embeddings;
DELETE source_embeddings WHERE in = $source;
DELETE embedding WHERE id IN $flat_embeddings;
DELETE $source;

RETURN count($flat_embeddings);

COMMIT;