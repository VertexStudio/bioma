use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
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

    /// Query to search for
    #[arg(short, long, default_value = "list ffmpeg dependencies")]
    query: String,
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
    let output_dir = engine.debug_output_dir()?;

    // Create indexer actor ID
    let indexer_id = ActorId::of::<Indexer>("/indexer");

    // Spawn and start the indexer actor
    let (mut indexer_ctx, mut indexer_actor) =
        Actor::spawn(engine.clone(), indexer_id.clone(), Indexer::default(), SpawnOptions::default()).await?;

    let indexer_handle = tokio::spawn(async move {
        if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
            error!("Indexer actor error: {}", e);
        }
    });

    // Create retriever actor ID
    let retriever_id = ActorId::of::<Retriever>("/retriever");

    // Spawn and start the retriever actor
    let (mut retriever_ctx, mut retriever_actor) =
        Actor::spawn(engine.clone(), retriever_id.clone(), Retriever::default(), SpawnOptions::default()).await?;

    let retriever_handle = tokio::spawn(async move {
        if let Err(e) = retriever_actor.start(&mut retriever_ctx).await {
            error!("Retriever actor error: {}", e);
        }
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor to connect to embeddings actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Get the workspace root
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
        vec![format!("{}/**/*.toml", workspace_root)]
    } else {
        args.globs.into_iter().map(|glob| format!("{}/{}", workspace_root, glob)).collect()
    };

    // Send globs to the indexer actor
    info!("Indexing");
    let index_globs = IndexGlobs { globs, ..Default::default() };
    let _indexer = relay_ctx
        .send::<Indexer, IndexGlobs>(
            index_globs,
            &indexer_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(500)).build(),
        )
        .await?;

    // Retrieve context
    info!("Retrieving context");
    let retrieve_context = RetrieveContext { query: args.query, limit: 10, threshold: 0.0 };
    let context = relay_ctx
        .send::<Retriever, RetrieveContext>(
            retrieve_context,
            &retriever_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
        )
        .await?;
    info!("Number of chunks: {}", context.context.len());

    // Save context to file for debugging
    let context_content = context.to_markdown();
    tokio::fs::write(output_dir.join("retriever_context.md"), context_content).await?;

    indexer_handle.abort();
    retriever_handle.abort();
    // Export the database for debugging
    dbg_export_db!(engine);

    Ok(())
}
