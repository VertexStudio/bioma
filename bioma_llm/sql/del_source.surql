BEGIN;

LET $source = source:{ source: $source };

LET $image_embeddings = SELECT ->{prefix}_source_embeddings.out as embedding FROM $source;

LET $flat_image_embeddings = array::flatten($image_embeddings.embedding);

DELETE {prefix}_model_embeddings WHERE out IN $flat_image_embeddings;
DELETE {prefix}_source_embeddings WHERE in = $source;
DELETE {prefix}_embedding WHERE id IN $flat_image_embeddings;
DELETE $source;

LET $total_deleted = count($flat_image_embeddings);

RETURN $total_deleted;

COMMIT;