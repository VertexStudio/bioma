-- Get the top k most similar embeddings for each source
SELECT 
    out.id AS id,
    out.text AS text,
    vector::similarity::cosine(out.embedding, $query) AS similarity,
    out.metadata as metadata,
    in.id.{source, uri} AS source
FROM type::table($prefix + "_source_embeddings")
WHERE 
    in.id.source IN $sources
    AND out.embedding <|{top_k}|> $query
ORDER BY similarity DESC;