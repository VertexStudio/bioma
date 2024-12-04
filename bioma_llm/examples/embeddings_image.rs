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

    tokio::spawn(async move {
        if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
            error!("Embeddings actor error: {}", e);
        }
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor to connect to embeddings actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // print cwd
    info!("Current working directory: {:?}", std::env::current_dir());

    // Image paths to embed
    let image_paths = vec!["assets/images/elephant.jpg", "assets/images/rust-pet.png", "assets/images/python-pet.jpg"]
        .iter()
        .map(|path| path.to_string())
        .collect::<Vec<String>>();

    // Store image embeddings
    let embeddings_ids = relay_ctx
        .send::<Embeddings, StoreEmbeddings>(
            StoreEmbeddings { content: EmbeddingContent::Image(image_paths.clone()), metadata: None },
            &embeddings_id,
            SendOptions::default(),
        )
        .await?;

    for id in embeddings_ids.ids.iter() {
        info!("Stored embeddings for image: {}", id);
    }

    // Search for similar images using an image query
    let top_k = embeddings::TopK {
        query: embeddings::Query::Image("assets/images/rust-pet.png".to_string()),
        threshold: 0.5,
        k: 3,
    };
    info!("Image query: {:?}", top_k);
    let similarities =
        relay_ctx.send::<Embeddings, embeddings::TopK>(top_k, &embeddings_id, SendOptions::default()).await?;

    for similarity in similarities {
        info!("Similarity score: {}", similarity.similarity);
        if let Some(metadata) = similarity.metadata {
            info!("Metadata: {:?}", metadata);
        }
    }

    // Generate embeddings without storing them
    let generated = relay_ctx
        .send::<Embeddings, GenerateEmbeddings>(
            GenerateEmbeddings { content: EmbeddingContent::Image(vec!["assets/images/rust-pet.png".to_string()]) },
            &embeddings_id,
            SendOptions::default(),
        )
        .await?;

    info!("Generated embedding dimensions: {}", generated.embeddings[0].len());

    // Export the database for debugging
    dbg_export_db!(engine);

    Ok(())
}
