BEGIN;

LET $source = source:{ source: $source, tag: $tag };

LET $image_embeddings = SELECT ->source_image_embeddings.out as embedding FROM $source;
LET $text_embeddings = SELECT ->source_text_embeddings.out as embedding FROM $source;

LET $flat_image_embeddings = array::flatten($image_embeddings.embedding);
LET $flat_text_embeddings = array::flatten($text_embeddings.embedding);

DELETE model_image_embeddings WHERE out IN $flat_image_embeddings;
DELETE source_image_embeddings WHERE in = $source;
DELETE image_embedding WHERE id IN $flat_image_embeddings;

DELETE model_text_embeddings WHERE out IN $flat_text_embeddings;
DELETE source_text_embeddings WHERE in = $source;
DELETE text_embedding WHERE id IN $flat_text_embeddings;

DELETE $source;

LET $total_deleted = count($flat_image_embeddings) + count($flat_text_embeddings);

RETURN $total_deleted;

COMMIT;