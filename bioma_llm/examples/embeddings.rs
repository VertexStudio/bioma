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
    let mut embeddings_actor = Actor::spawn(
        &engine,
        &embeddings_id,
        Embeddings { model_name: "nomic-embed-text".to_string(), generation_options: Default::default(), ollama: None },
    )
    .await?;

    let embeddings_handle = tokio::spawn(async move {
        if let Err(e) = embeddings_actor.start().await {
            error!("Embeddings actor error: {}", e);
        }
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Create a bridge actor to connect the rerank actor
    let bridge_id = ActorId::of::<BridgeActor>("/bridge");
    let bridge_actor = Actor::spawn(&engine, &bridge_id, BridgeActor).await?;

    // Text to embed
    let text = "Hello, how are you?";

    // Send the text to the embeddings actor
    let embeddings = bridge_actor
        .send::<Embeddings, GenerateEmbeddings>(
            GenerateEmbeddings { text: text.to_string() },
            &embeddings_id,
            SendOptions::default(),
        )
        .await?;

    info!("Embeddings: {:?}", embeddings);

    embeddings_handle.abort();

    Ok(())
}
