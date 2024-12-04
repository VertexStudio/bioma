-- Get the top k most similar embeddings to the query
-- $source is the regex pattern to match against
SELECT
    id,
    text,
    vector::similarity::cosine(embedding, $query) AS similarity,
    metadata
FROM type::table($prefix + "_embedding")
WHERE embedding <|{top_k}|> $query
-- AND string::matches(source, $source)
-- ORDER BY similarity DESC;