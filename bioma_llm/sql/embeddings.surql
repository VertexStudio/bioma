LET $emb = CREATE ONLY embedding SET text = $text, embedding = $embedding;
RELATE $model_id->model_embeddings->($emb.id);