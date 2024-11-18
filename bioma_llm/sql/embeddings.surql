LET $id = rand::ulid(time::now());
LET $emb = CREATE ONLY embedding:{id: $id, tag: $tag} SET text = $text, embedding = $embedding, metadata = $metadata;
RELATE $model_id->model_embeddings->($emb.id);

LET $src = SELECT VALUE id FROM source:{source: $source, tag: $tag} LIMIT 1;
LET $src_id = IF array::len($src) = 0 THEN 
    (CREATE source:{source: $source, tag: $tag} SET source = $source).id 
ELSE 
    $src[0]
END;

RELATE $src_id->source_embeddings->($emb.id);