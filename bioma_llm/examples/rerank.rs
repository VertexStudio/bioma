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

    // Create actor IDs
    let rerank_id = ActorId::of::<Rerank>("/rerank");

    // Spawn and start the rerank actor
    let (mut rerank_ctx, mut rerank_actor) =
        Actor::spawn(engine.clone(), rerank_id.clone(), Rerank::default(), SpawnOptions::default()).await?;
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
        "The weather in Tokyo is sunny today.".to_string(),
        "Tokyo's climate is generally mild.".to_string(),
        "What's the temperature in Tokyo right now?".to_string(),
        "Tokyo experiences four distinct seasons.".to_string(),
        "Is it raining in Tokyo at the moment?".to_string(),
        "Tokyo's weather forecast for the week".to_string(),
        "The best time to visit Tokyo for good weather".to_string(),
        "How does Tokyo's weather compare to other cities?".to_string(),
        "Tokyo's average temperature by month".to_string(),
        "Does it snow in Tokyo?".to_string(),
        "Tokyo's humidity levels throughout the year".to_string(),
        "The impact of typhoons on Tokyo's weather".to_string(),
        "Tokyo's air quality and weather conditions".to_string(),
        "How climate change affects Tokyo's weather patterns".to_string(),
        "Tokyo's record high and low temperatures".to_string(),
        "The best outdoor activities in Tokyo based on weather".to_string(),
        "Tokyo's weather during cherry blossom season".to_string(),
        "How to dress for Tokyo's weather in different seasons".to_string(),
        "Tokyo's rainfall patterns and monsoon season".to_string(),
        "The effect of urban heat islands on Tokyo's weather".to_string(),
        "Tokyo's weather-related natural disasters".to_string(),
        "How Tokyo's weather affects public transportation".to_string(),
        "The influence of Mount Fuji on Tokyo's weather".to_string(),
        "Tokyo's weather forecasting technology and accuracy".to_string(),
        "How Tokyo's weather impacts energy consumption".to_string(),
    ];

    // Query to use for reranking
    let query = "What is the weather in Tokyo?";

    // Send the texts to the rerank actor
    let ranked_texts = relay_ctx
        .send_and_wait_reply::<Rerank, RankTexts>(
            RankTexts { query: query.to_string(), texts: texts.clone() },
            &rerank_id,
            SendOptions::default(),
        )
        .await?;

    info!("Ranked texts: {:?}", ranked_texts);

    // Clone the ranked_texts so we can sort them
    let mut sorted_texts = ranked_texts.clone();

    // Sort the texts by score in descending order
    sorted_texts.texts.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    println!("Query: {}", query);
    for ranked_text in sorted_texts.texts {
        println!("{:>2}: {:>6.2} {}", ranked_text.index, ranked_text.score, &texts[ranked_text.index]);
    }

    rerank_handle.abort();

    // Export the database for debugging
    dbg_export_db!(engine);

    Ok(())
}
