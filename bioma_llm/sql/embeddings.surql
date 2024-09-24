LET $id = rand::ulid(time::now());
LET $emb = CREATE ONLY embedding:{id: $id, tag: $tag} SET text = $text, embedding = $embedding, metadata = $metadata;
RELATE $model_id->model_embeddings->($emb.id);
LET $src = CREATE ONLY source:{tag: $tag, source: $source} SET source = $source;
RELATE ($src.id)->source_embeddings->($emb.id);
