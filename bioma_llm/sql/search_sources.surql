-- First, filter the sources and store the result in a variable
LET $filtered_ids = (
    SELECT ->{prefix}_source_embeddings.out.id AS embeddings
    FROM source
    WHERE id.source IN $sources
);

-- Then, use the filtered result in the main query
SELECT 
    id,
    text,
    vector::similarity::cosine(embedding, $query) AS similarity,
    metadata,
    <-{prefix}_source_embeddings.in[0].id.{source, uri} AS source
FROM type::table($prefix + "_embedding")
WHERE 
    id IN array::flatten($filtered_ids.embeddings)
    AND embedding <|{top_k}|> $query
ORDER BY similarity DESC;