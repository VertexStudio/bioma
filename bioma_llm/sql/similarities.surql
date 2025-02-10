-- First get the filtered embeddings by regex pattern
LET $filtered_embeddings = array::flatten((
    SELECT ->{prefix}_source_embeddings.out.id AS embeddings
    FROM source 
    WHERE id.source @@ $source
).embeddings);

-- Then perform vector similarity search on the filtered subset
SELECT 
    id,
    text,
    vector::similarity::cosine(embedding, $query) AS similarity,
    metadata,
    <-{prefix}_source_embeddings.in[0].id.{source, uri} AS source
FROM type::table($prefix + "_embedding")
WHERE 
    -- First apply the regex filter
    id IN $filtered_embeddings
    -- Then do vector similarity search on filtered results
    AND embedding <|{top_k}|> $query
ORDER BY similarity DESC;