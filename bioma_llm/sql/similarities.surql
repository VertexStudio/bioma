-- Get the top k most similar embeddings to the query
-- $source is the regex pattern to match against
SELECT
    id,
    text,
    vector::similarity::cosine(embedding, $query) AS similarity,
    metadata
FROM type::table($prefix + "_embedding")
WHERE embedding <|{top_k}|> $query
AND id IN (
    SELECT out AS embeddings
    FROM type::table($prefix + "_source_embeddings")
    WHERE string::matches(in.id.uri, $source)
).embeddings
ORDER BY similarity DESC