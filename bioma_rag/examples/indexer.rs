use bioma_actor::prelude::*;
use bioma_rag::prelude::*;
use clap::Parser;
use tracing::{error, info};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Root folder to index
    #[arg(short, long)]
    root: Option<String>,

    /// Globs to index (can be specified multiple times)
    #[arg(short, long)]
    globs: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Parse command-line arguments
    let args = Args::parse();

    // Initialize the actor system
    let engine = Engine::test().await?;

    // Create indexer actor ID
    let indexer_id = ActorId::of::<Indexer>("/indexer");

    // Spawn and start the indexer actor
    let (mut indexer_ctx, mut indexer_actor) =
        Actor::spawn(engine.clone(), indexer_id.clone(), Indexer::default(), SpawnOptions::default()).await?;

    let _indexer_handle = tokio::spawn(async move {
        if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
            error!("Indexer actor error: {}", e);
        }
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor to send messages to other actors
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Get root for indexer or default to workspace root
    let workspace_root = args.root.unwrap_or_else(|| {
        std::env::var("CARGO_MANIFEST_DIR")
            .map(std::path::PathBuf::from)
            .ok()
            .and_then(|path| path.parent().map(|p| p.to_path_buf()))
            .unwrap()
            .to_string_lossy()
            .to_string()
    });

    // Prepare globs
    let globs = if args.globs.is_empty() {
        vec![format!("{}/bioma_actor/**/*.rs", workspace_root)]
    } else {
        args.globs.into_iter().map(|glob| format!("{}/{}", workspace_root, glob)).collect()
    };

    // Send globs to the indexer actor
    info!("Indexing");
    let index_globs =
        Index::builder().content(IndexContent::Globs(GlobsContent::builder().globs(globs).build())).build();

    let _indexer = relay_ctx
        .send_and_wait_reply::<Indexer, Index>(
            index_globs,
            &indexer_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(500)).build(),
        )
        .await?;

    // Export the database for debugging
    dbg_export_db!(engine);

    Ok(())
}
