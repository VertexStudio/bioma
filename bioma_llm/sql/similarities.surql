-- Get the top k most similar embeddings to the query
-- $source is the regex pattern to match against
SELECT * FROM (
    SELECT
        out.text AS text,
        vector::similarity::cosine(out.embedding, $query) AS similarity,
        out.metadata AS metadata
    FROM type::table($prefix + "_source_embeddings")
    WHERE string::matches(in.source, $source)
) WHERE (similarity > $threshold)
ORDER BY similarity DESC
LIMIT $top_k;
