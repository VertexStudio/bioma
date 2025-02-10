SELECT 
    id,
    text,
    vector::similarity::cosine(embedding, $query) AS similarity,
    metadata,
    <-{prefix}_source_embeddings.in[0].id.{source, uri} AS source
FROM type::table($prefix + "_embedding")
WHERE 
    -- First apply the regex filter
    id IN array::flatten(((SELECT ->{prefix}_source_embeddings.out.id AS embeddings
    FROM source 
    WHERE id.source IN $sources).embeddings))
    -- Then do vector similarity search on filtered results
    AND embedding <|{top_k}|> $query
ORDER BY similarity DESC;