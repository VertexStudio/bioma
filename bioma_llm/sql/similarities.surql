SELECT * FROM (
    SELECT
        out.text AS text,
        vector::similarity::cosine(out.embedding, $query) AS similarity,
        out.metadata AS metadata,
        out.id.tag AS tag
    FROM type::table($prefix + "_source_embeddings")
    WHERE string::matches(in.source, $pattern)
) WHERE (tag IS NONE OR tag = $tag)
  AND (similarity > $threshold)
ORDER BY similarity DESC
LIMIT $top_k;