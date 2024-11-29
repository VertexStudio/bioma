LET $embeddings = IF array::len($sources) > 0 {
    // If sources provided, get filtered embeddings
    RETURN SELECT VALUE out.* 
    FROM type::table($prefix + "_source_embeddings") 
    WHERE in.source INSIDE $sources
} ELSE {
    // If no sources, get all embeddings
    RETURN SELECT * FROM type::table($prefix + "_embedding")
};

// Perform similarity search on either filtered or all embeddings
SELECT * FROM (
    SELECT 
        text,
        vector::similarity::cosine(embedding, $query) AS similarity,
        metadata,
        id.tag AS tag
    FROM $embeddings
) WHERE (tag IS NONE OR tag = $tag)
    AND (similarity > $threshold)
ORDER BY similarity DESC
LIMIT $top_k;