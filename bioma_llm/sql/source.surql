LET $src_id = (CREATE ONLY source:{source: $source, uri: $uri} SET summary = $summary, video_url = $video_url).id;
FOR $emb_id IN type::array($emb_ids) {
    RELATE (type::thing($src_id))->(type::table($prefix + "_source_embeddings"))->(type::thing($emb_id));
}