SELECT * FROM (
    SELECT
        text,
        vector::similarity::cosine(embedding, $query) AS similarity,
        id.tag AS tag
    FROM embedding
) WHERE
    (tag IS NONE OR tag = $tag)
    AND (similarity > $threshold)
ORDER BY similarity DESC
LIMIT $top_k
