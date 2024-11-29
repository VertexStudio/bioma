SELECT * FROM (
    SELECT
        text,
        vector::similarity::cosine(embedding, $query) AS similarity,
        metadata,
        id.tag AS tag,
        ->source_embeddings.in.source AS source
    FROM embedding
) WHERE
    (tag IS NONE OR tag = $tag)
    AND (similarity > $threshold)
    AND ($sources IS NONE OR source INSIDE $sources)
ORDER BY similarity DESC
LIMIT $top_k