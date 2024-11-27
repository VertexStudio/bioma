SELECT * FROM (
    SELECT
        text,
        vector::similarity::cosine(embedding, $query) AS similarity,
        metadata,
        id.tag AS tag
    FROM type::table($prefix + "_embedding")
) WHERE
    (tag IS NONE OR tag = $tag)
    AND (similarity > $threshold)
ORDER BY similarity DESC
LIMIT $top_k