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
    let (mut embeddings_ctx, mut embeddings_actor) = Actor::spawn(
        engine.clone(),
        embeddings_id.clone(),
        Embeddings { model_name: "nomic-embed-text".to_string(), ..Default::default() },
        SpawnOptions::default(),
    )
    .await?;

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

    // Text to embed
    let text = "Hello, how are you?";

    // Send the texts to the embeddings actor
    let embeddings = relay_ctx
        .send::<Embeddings, GenerateEmbeddings>(
            GenerateEmbeddings { texts: vec![text.to_string()], store: true },
            &embeddings_id,
            SendOptions::default(),
        )
        .await?;

    info!("Embeddings: {:?}", embeddings);

    embeddings_handle.abort();

    dbg_export_db!(engine);

    Ok(())
}
