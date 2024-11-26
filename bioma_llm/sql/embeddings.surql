LET $id = rand::ulid(time::now());
LET $emb = CREATE ONLY type::table($table) CONTENT {
    id: $id, 
    tag: $tag,
    text: $text, 
    embedding: $embedding, 
    metadata: $metadata
};
RELATE $model_id->model_embeddings->($emb.id);

LET $src = SELECT VALUE id FROM ONLY source:{source: $source, tag: $tag};
LET $src_id = IF $src == NONE THEN 
    (CREATE ONLY source:{source: $source, tag: $tag} SET source = $source).id 
ELSE 
    $src
END;

RELATE $src_id->source_embeddings->($emb.id);