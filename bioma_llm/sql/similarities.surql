LET $embeddings = IF array::len($sources) > 0 {
    // If sources provided, get filtered embeddings
    RETURN SELECT * FROM (
        SELECT VALUE {
            text: out.text, 
            similarity: vector::similarity::cosine(out.embedding, $query),
            metadata: out.metadata,
            tag: out.id.tag
        }
        FROM type::table($prefix + "_source_embeddings") 
        WHERE in.source INSIDE $sources
    ) WHERE (tag IS NONE OR tag = $tag) AND (similarity > $threshold)
} ELSE {
    // If no sources, get all embeddings
    RETURN SELECT * FROM (
        SELECT VALUE {
            text: text,
            similarity: vector::similarity::cosine(embedding, $query),
            metadata: metadata,
            tag: tag
        } 
        FROM type::table($prefix + "_embedding")
    ) WHERE (tag IS NONE OR tag = $tag) AND (similarity > $threshold)
};