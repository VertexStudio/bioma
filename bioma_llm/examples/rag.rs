use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use retriever::RetrieveQuery;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Initialize the actor system
    let engine = Engine::test().await?;
    let output_dir = engine.output_dir();

    let query = "identify the most important dependency found in cargo workspace";

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

    // Create retriever actor ID
    let retriever_id = ActorId::of::<Retriever>("/retriever");

    // Spawn and start the retriever actor
    let (mut retriever_ctx, mut retriever_actor) =
        Actor::spawn(engine.clone(), retriever_id.clone(), Retriever::default(), SpawnOptions::default()).await?;

    let _retriever_handle = tokio::spawn(async move {
        if let Err(e) = retriever_actor.start(&mut retriever_ctx).await {
            error!("Retriever actor error: {}", e);
        }
    });

    // Create chat conversation
    let mut conversation = vec![];

    let ask = Chat::builder().model("llama3.2".into()).messages_number_limit(10).history(conversation.clone()).build();

    let ask_id = ActorId::of::<Chat>("/ask");
    let (mut ask_ctx, mut ask_actor) =
        Actor::spawn(engine.clone(), ask_id.clone(), ask, SpawnOptions::default()).await?;

    let _ask_handle = tokio::spawn(async move {
        if let Err(e) = ask_actor.start(&mut ask_ctx).await {
            error!("Ask actor error: {}", e);
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
    let index_globs = IndexGlobs::builder()
        .globs(vec![
            // workspace_root.clone() + "/**/*.surql",
            workspace_root.clone() + "/**/*.toml",
        ])
        .build();
    let _indexer = relay_ctx
        .send_and_wait_reply::<Indexer, IndexGlobs>(
            index_globs,
            &indexer_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(500)).build(),
        )
        .await?;

    // Retrieve context
    info!("Retrieving context");
    let retrieve_context =
        RetrieveContext { query: RetrieveQuery::Text(query.to_string()), limit: 10, threshold: 0.0, source: None };
    let context = relay_ctx
        .send_and_wait_reply::<Retriever, RetrieveContext>(
            retrieve_context,
            &retriever_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
        )
        .await?;
    info!("Number of chunks: {}", context.context.len());

    // Save context to file for debugging
    let context_content = context.to_markdown();
    tokio::fs::write(output_dir.join("debug").join("rag_context.md"), &context_content).await?;

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
    info!("Sending context to ask actor");
    let ask_response = relay_ctx
        .send_and_wait_reply::<Chat, ChatMessages>(
            ChatMessages {
                messages: conversation.clone(),
                restart: false,
                persist: false,
                stream: false,
                format: None,
            },
            &ask_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(500)).build(),
        )
        .await?;
    info!("Ask {} responded", &ask_response.model);

    // Save chat to file for debugging
    let mut ask_content = String::new();
    for message in &conversation {
        ask_content.push_str(&format!("{:?}: {}\n\n", message.role, message.content));
    }
    let response = ask_response.message.unwrap();
    ask_content.push_str(&format!("{:?}: {}\n\n", &response.role, &response.content));
    tokio::fs::write(output_dir.join("debug").join("rag_ask.md"), ask_content).await?;

    // Export the database for debugging
    dbg_export_db!(engine);

    Ok(())
}
