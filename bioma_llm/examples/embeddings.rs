use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Initialize the actor system
    let engine = Engine::test().await?;

    // Create actor ID
    let embeddings_id = ActorId::of::<Embeddings>("/embeddings");

    // Spawn and start the embeddings actor
    let (mut embeddings_ctx, mut embeddings_actor) =
        Actor::spawn(engine.clone(), embeddings_id.clone(), Embeddings::default(), SpawnOptions::default()).await?;

    let embeddings_handle = tokio::spawn(async move {
        if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
            error!("Embeddings actor error: {}", e);
        }
    });

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
    ]
    .iter()
    .map(|text| text.to_string())
    .collect::<Vec<String>>();

    // Send the texts to the embeddings actor
    let embeddings_lentghs = relay_ctx
        .send::<Embeddings, StoreTextEmbeddings>(
            StoreTextEmbeddings {
                source: "test".to_string(),
                texts: texts.clone(),
                metadata: None,
                tag: Some("test".to_string()),
            },
            &embeddings_id,
            SendOptions::default(),
        )
        .await?;

    for (i, length) in embeddings_lentghs.lengths.iter().enumerate() {
        info!("Stored embeddings for text: {} with length: {}", texts[i], length);
    }

    // Get similarities
    let top_k = embeddings::TopK {
        query: embeddings::Query::Text("Hello, how are you?".to_string()),
        threshold: -0.5,
        k: 5,
        tag: Some("test".to_string()),
    };
    info!("Query: {:?}", top_k);
    let similarities =
        relay_ctx.send::<Embeddings, embeddings::TopK>(top_k, &embeddings_id, SendOptions::default()).await?;

    for similarity in similarities {
        info!("Similarity: {}   {}", similarity.text, similarity.similarity);
    }

    embeddings_handle.abort();

    // Export the database for debugging
    dbg_export_db!(engine);

    Ok(())
}
