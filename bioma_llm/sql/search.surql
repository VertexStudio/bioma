-- Get the top k most similar embeddings to the query
SELECT
    id,
    text,
    vector::similarity::cosine(embedding, $query) AS similarity,
    metadata,
    <-{prefix}_source_embeddings.in[0].id.{source, uri} AS source
FROM type::table($prefix + "_embedding")
WHERE embedding <|{top_k}|> $query
ORDER BY similarity DESC