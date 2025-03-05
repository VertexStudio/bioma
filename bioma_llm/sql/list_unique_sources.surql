SELECT 
    id.source AS source,
    count() as count
FROM source
GROUP BY
    source;