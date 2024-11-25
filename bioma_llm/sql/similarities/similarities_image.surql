SELECT * FROM (
    SELECT
        caption,
        vector::similarity::cosine(embedding, $query) AS similarity,
        metadata,
        id.tag AS tag
    FROM image_embedding
) WHERE
    (tag IS NONE OR tag = $tag)
    AND (similarity > $threshold)
ORDER BY similarity DESC
LIMIT $top_k