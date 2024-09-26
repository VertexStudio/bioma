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

    let query = "identify the most important dependency found in cargo workspace";

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

    // Create chat conversation
    let mut conversation = vec![];

    let chat = Chat::builder().model("llama3.2".into()).messages_number_limit(10).history(conversation.clone()).build();

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

    // Retrieve context
    info!("Retrieving context");
    let retrieve_context = RetrieveContext { query: query.to_string(), limit: 10, threshold: 0.0 };
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
    tokio::fs::write(output_dir.join("rag_context.md"), &context_content).await?;

    // Add context to conversation as a system message
    let context_message = ChatMessage::system(
        "You are a helpful programming assistant. Use the following context to answer the user's query: \n\n"
            .to_string()
            + &context_content,
    );
    conversation.push(context_message);

    // Add user's query to conversation
    let user_query = ChatMessage::user(query.to_string());
    conversation.push(user_query);

    // Send the context to the chat actor
    info!("Sending context to chat actor");
    let chat_response = relay_ctx
        .send::<Chat, ChatMessages>(
            ChatMessages { messages: conversation.clone(), restart: false },
            &chat_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(500)).build(),
        )
        .await?;
    info!("Chat {} responded", &chat_response.model);

    // Save chat to file for debugging
    let mut chat_content = String::new();
    for message in &conversation {
        chat_content.push_str(&format!("{:?}: {}\n\n", message.role, message.content));
    }
    let response = chat_response.message.unwrap();
    chat_content.push_str(&format!("{:?}: {}\n\n", &response.role, &response.content));
    tokio::fs::write(output_dir.join("rag_chat.md"), chat_content).await?;

    indexer_handle.abort();
    retriever_handle.abort();
    chat_handle.abort();

    // Export the database for debugging
    dbg_export_db!(engine);

    Ok(())
}
