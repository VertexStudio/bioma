LET $id = rand::ulid(time::now());
LET $emb = CREATE ONLY type::table($prefix + "_embedding") CONTENT {
    id: $id, 
    text: $text, 
    embedding: $embedding,
    metadata: $metadata
};
RELATE $model_id->(type::table($prefix + "_model_embeddings"))->($emb.id);
RETURN $emb.id;

-- LET $src = SELECT VALUE id FROM ONLY source:{source: $source, uri: $uri};
-- LET $src_id = IF $src == NONE THEN 
--     (CREATE ONLY source:{source: $source, uri: $uri} SET source = $source, uri = $uri).id 
-- ELSE 
--     $src
-- END;

-- RELATE $src_id->(type::table($prefix + "_source_embeddings"))->($emb.id);