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
    let mut rerank_actor =
        Actor::spawn(&engine, &rerank_id, Rerank { url: Url::parse("http://localhost:9124/rerank").unwrap() }).await?;
    let rerank_handle = tokio::spawn(async move {
        if let Err(e) = rerank_actor.start().await {
            error!("Rerank actor error: {}", e);
        }
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Create a bridge actor to connect the rerank actor
    let bridge_id = ActorId::of::<BridgeActor>("/bridge");
    let bridge_actor = Actor::spawn(&engine, &bridge_id, BridgeActor).await?;

    // Texts to rerank
    let texts: Vec<String> = vec![
        "Hello, how are you?".to_string(),
        "What is the weather in Tokyo?".to_string(),
        "Can you recommend a good book?".to_string(),
    ];

    // Query to use for reranking
    let query = "What is the weather in Tokyo?";

    // Send the texts to the rerank actor
    let ranked_texts = bridge_actor
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
