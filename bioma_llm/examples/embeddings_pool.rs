use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use futures::future::join_all;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Initialize the actor system
    let engine = Engine::test().await?;

    // Create multiple embeddings actors
    let num_embeddings_actors = 3;
    let mut embeddings_actors = Vec::new();
    let mut embeddings_handles = Vec::new();

    for i in 0..num_embeddings_actors {
        let embeddings_id = ActorId::of::<Embeddings>(format!("/embeddings_{}", i));
        let (mut embeddings_ctx, mut embeddings_actor) =
            Actor::spawn(engine.clone(), embeddings_id.clone(), Embeddings::default(), SpawnOptions::default()).await?;

        let embeddings_handle = tokio::spawn(async move {
            if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
                error!("Embeddings actor {} error: {}", i, e);
            }
        });

        embeddings_actors.push(embeddings_id);
        embeddings_handles.push(embeddings_handle);
    }

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor to connect to embeddings actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Texts to embed
    let texts = vec![
        "Hello, how are you?",
        "What is the meaning of life?",
        "The quick brown fox jumps over the lazy dog",
        "Why is the sky blue?",
        "What is the capital of the moon?",
        "How are they doing?",
        "Are you ok?",
        "What's your favorite color?",
        "Can you explain quantum physics?",
        "Where do dreams come from?",
        "How does the internet work?",
        "What's the best way to learn a new language?",
        "Why do leaves change color in autumn?",
        "What's the difference between a virus and a bacteria?",
        "How do airplanes fly?",
        "What causes earthquakes?",
        "How does photosynthesis work?",
    ]
    .iter()
    .map(|text| text.to_string())
    .collect::<Vec<String>>();

    // Distribute texts among embeddings actors
    let chunks: Vec<Vec<String>> = texts
        .chunks((texts.len() + num_embeddings_actors - 1) / num_embeddings_actors)
        .map(|chunk| chunk.to_vec())
        .collect();

    let mut embedding_futures = Vec::new();

    for (i, chunk) in chunks.iter().enumerate() {
        let embeddings_id = &embeddings_actors[i];
        let future = relay_ctx.send::<Embeddings, StoreEmbeddings>(
            StoreEmbeddings {
                source: "test".to_string(),
                content: EmbeddingContent::Text(chunk.clone()),
                metadata: None,
                tag: Some(format!("test_{}", i)),
            },
            embeddings_id,
            SendOptions::default(),
        );
        embedding_futures.push(future);
    }

    let all_embeddings = join_all(embedding_futures).await;

    for (i, embeddings_result) in all_embeddings.into_iter().enumerate() {
        let embeddings_lentghs = embeddings_result?;
        for (j, length) in embeddings_lentghs.lengths.iter().enumerate() {
            info!("Actor {}: Stored embeddings for text: {} with length: {}", i, chunks[i][j], length);
        }
    }

    // Get similarities from all actors
    let mut similarity_futures = Vec::new();

    for (i, embeddings_id) in embeddings_actors.iter().enumerate() {
        let top_k = embeddings::TopK {
            query: embeddings::Query::Text("Hello, how are you?".to_string()),
            threshold: -0.5,
            k: 5,
            tag: Some(format!("test_{}", i)),
            sources: None,
        };
        info!("Query for actor {}: {:?}", i, top_k);
        let future = relay_ctx.send::<Embeddings, embeddings::TopK>(top_k, embeddings_id, SendOptions::default());
        similarity_futures.push(future);
    }

    let all_similarities = join_all(similarity_futures).await;

    for (i, similarities_result) in all_similarities.into_iter().enumerate() {
        let similarities = similarities_result?;
        for similarity in similarities {
            info!("Actor {}: Similarity: {:?}   {}", i, similarity.text, similarity.similarity);
        }
    }

    // Abort all embeddings actors
    for handle in embeddings_handles {
        handle.abort();
    }

    // Export the database for debugging
    dbg_export_db!(engine);

    Ok(())
}
