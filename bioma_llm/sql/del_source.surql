BEGIN;

LET $source = source:{ source: $source, tag: $tag };

LET $embeddings = SELECT ->source_embeddings.out as embedding FROM $source;

LET $flat_embeddings = array::flatten($embeddings.embedding);

DELETE model_embeddings WHERE out IN $flat_embeddings;
DELETE source_embeddings WHERE in = $source;
DELETE embedding WHERE id IN $flat_embeddings;
DELETE $source;

RETURN count($flat_embeddings);

COMMIT;