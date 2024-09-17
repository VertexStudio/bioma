LET $emb = CREATE ONLY embedding:{id: $id, tag: $tag} SET text = $text, embedding = $embedding;
RELATE $model_id->model_embeddings->($emb.id);