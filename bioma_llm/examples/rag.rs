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
    let output_dir = engine.debug_output_dir()?;

    let query = "list ffmpeg dependencies";

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

    // Create chat pre-history
    let history = vec![
        ChatMessage::system("You are a helpful programming assistant".into()),
        ChatMessage::user("Hello, how are you?".into()),
        ChatMessage::assistant("I'm doing well, thank you! How can I help you today?".into()),
        ChatMessage::user(query.into()),
    ];

    // Create chat actor with pre-history
    let chat = Chat {
        model_name: "gemma2:2b".to_string(),
        generation_options: Default::default(),
        messages_number_limit: 10,
        history: history.clone(),
        ollama: None,
    };

    let chat_id = ActorId::of::<Chat>("/chat");
    let (mut chat_ctx, mut chat_actor) =
        Actor::spawn(engine.clone(), chat_id.clone(), chat, SpawnOptions::default()).await?;

    let chat_handle = tokio::spawn(async move {
        if let Err(e) = chat_actor.start(&mut chat_ctx).await {
            error!("Chat actor error: {}", e);
        }
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor to connect to embeddings actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Get the workspace root
    let workspace_root = std::env::var("CARGO_MANIFEST_DIR")
        .map(std::path::PathBuf::from)
        .ok()
        .and_then(|path| path.parent().map(|p| p.to_path_buf()))
        .unwrap()
        .to_string_lossy()
        .to_string();

    // Send globs to the indexer actor
    info!("Indexing");
    let index_globs = IndexGlobs {
        globs: vec![
            // workspace_root.clone() + "/**/*.surql",
            workspace_root.clone() + "/**/*.toml",
        ],
        ..Default::default()
    };
    let _indexer = relay_ctx
        .send::<Indexer, IndexGlobs>(
            index_globs,
            &indexer_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(500)).build(),
        )
        .await?;

    // Fetch context
    info!("Fetching context");
    let fetch_context = RetrieveContext { query: query.to_string(), limit: 10, threshold: 0.0 };
    let context = relay_ctx
        .send::<Retriever, RetrieveContext>(
            fetch_context,
            &indexer_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
        )
        .await?;
    info!("Number of chunks: {}", context.context.len());

    // Save context to file for debugging
    tokio::fs::write(output_dir.join("rag_context.md"), context.context.join("\n\n")).await?;

    // Send the context to the chat actor
    info!("Sending context to chat actor");
    let chat_message = ChatMessage::system("Context to answer user query: ".to_string() + &context.context.join("\n"));
    let chat_response = relay_ctx
        .send::<Chat, ChatMessage>(
            chat_message,
            &chat_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(500)).build(),
        )
        .await?;
    info!("Chat {} responded", &chat_response.model);

    // Save chat to file for debugging
    let mut chat_content = String::new();
    for message in &history {
        chat_content.push_str(&format!("{:?}: {}\n\n", message.role, message.content));
    }
    let response = chat_response.message.unwrap();
    chat_content.push_str(&format!("{:?}: {}\n\n", &response.role, &response.content));
    tokio::fs::write(output_dir.join("rag_chat.md"), chat_content).await?;

    indexer_handle.abort();
    chat_handle.abort();

    // Export the database for debugging
    dbg_export_db!(engine);

    Ok(())
}
