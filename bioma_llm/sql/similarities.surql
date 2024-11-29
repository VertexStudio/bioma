SELECT * FROM (
    SELECT
        text,
        vector::similarity::cosine(embedding, $query) AS similarity,
        metadata,
        id.tag AS tag,
        <-{prefix}_source_embeddings.in.source AS source
    FROM type::table($prefix + "_embedding")
) WHERE
    (tag IS NONE OR tag = $tag)
    AND (
        $sources IS NONE 
        OR source INSIDE $sources
    )
    AND (similarity > $threshold)
ORDER BY similarity DESC
LIMIT $top_k