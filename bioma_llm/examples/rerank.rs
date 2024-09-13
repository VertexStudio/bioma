use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use tracing::{error, info};
use url::Url;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Initialize the actor system
    let engine = Engine::test().await?;

    // Create actor IDs
    let rerank_id = ActorId::of::<Rerank>("/rerank");

    // Spawn and start the rerank actor
    let (mut rerank_ctx, mut rerank_actor) = Actor::spawn(
        engine.clone(),
        rerank_id.clone(),
        Rerank { url: Url::parse("http://localhost:9124/rerank").unwrap() },
        SpawnOptions::default(),
    )
    .await?;
    let rerank_handle = tokio::spawn(async move {
        if let Err(e) = rerank_actor.start(&mut rerank_ctx).await {
            error!("Rerank actor error: {}", e);
        }
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor to connect to rerank actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Texts to rerank
    let texts: Vec<String> = vec![
        "Hello, how are you?".to_string(),
        "What is the weather in Tokyo?".to_string(),
        "Can you recommend a good book?".to_string(),
    ];

    // Query to use for reranking
    let query = "What is the weather in Tokyo?";

    // Send the texts to the rerank actor
    let ranked_texts = relay_ctx
        .send::<Rerank, RankTexts>(
            RankTexts { query: query.to_string(), texts: texts, raw_scores: false },
            &rerank_id,
            SendOptions::default(),
        )
        .await?;

    info!("Ranked texts: {:?}", ranked_texts);

    rerank_handle.abort();

    Ok(())
}
