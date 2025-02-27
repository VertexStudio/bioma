-- Get filtered sources
LET $filtered_ids = array::flatten((
    SELECT VALUE ->{prefix}_source_embeddings.out.id 
    FROM source
    WHERE id.source IN $sources
));

-- Get the top k most similar embeddings to the query from the filtered IDs
SELECT 
    id,
    text,
    vector::similarity::cosine(embedding, $query) AS similarity,
    metadata,
    <-{prefix}_source_embeddings.in[0].id.{source, uri} AS source,
    <-{prefix}_source_embeddings.in[0].video_url AS video_url
FROM type::table($prefix + "_embedding")
WHERE 
    id IN $filtered_ids
    AND embedding <|{top_k}|> $query
ORDER BY similarity DESC;