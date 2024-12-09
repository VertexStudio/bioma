use bioma_actor::prelude::*;
use bioma_llm::rerank::{RankTexts, Rerank};
use futures::future::join_all;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Initialize the actor system
    let engine = Engine::test().await?;

    // Create multiple rerank actors
    let num_rerank_actors = 3;
    let mut rerank_actors = Vec::new();
    let mut rerank_handles = Vec::new();

    for i in 0..num_rerank_actors {
        let rerank_id = ActorId::of::<Rerank>(format!("/rerank_{}", i));
        let (mut rerank_ctx, mut rerank_actor) =
            Actor::spawn(engine.clone(), rerank_id.clone(), Rerank::default(), SpawnOptions::default()).await?;

        let rerank_handle = tokio::spawn(async move {
            if let Err(e) = rerank_actor.start(&mut rerank_ctx).await {
                error!("Rerank actor {} error: {}", i, e);
            }
        });

        rerank_actors.push(rerank_id);
        rerank_handles.push(rerank_handle);
    }

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor to connect to rerank actors
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Queries and texts to rerank
    let queries = vec![
        "What is the capital of France?",
        "Who wrote Romeo and Juliet?",
        "What is the largest planet in our solar system?",
        "Who painted the Mona Lisa?",
        "What is the boiling point of water?",
    ];

    let texts = vec![
        "Paris is the capital of France.",
        "London is the capital of England.",
        "William Shakespeare wrote Romeo and Juliet.",
        "Charles Dickens wrote Great Expectations.",
        "Jupiter is the largest planet in our solar system.",
        "Mars is known as the Red Planet.",
        "Leonardo da Vinci painted the Mona Lisa.",
        "Vincent van Gogh painted The Starry Night.",
        "Water boils at 100 degrees Celsius at sea level.",
        "The freezing point of water is 0 degrees Celsius.",
        "The Earth rotates on its axis once every 24 hours.",
        "The Nile is the longest river in the world.",
        "Mount Everest is the highest peak on Earth.",
        "The Great Wall of China is visible from space.",
        "The human body is made up of approximately 60% water.",
        "Photosynthesis is the process by which plants make their food.",
        "The Sahara is the largest hot desert in the world.",
        "Gravity is the force that attracts objects towards each other.",
        "The speed of light is approximately 299,792,458 meters per second.",
        "DNA stands for Deoxyribonucleic Acid.",
        "The Eiffel Tower is located in Paris, France.",
        "The Amazon rainforest produces about 20% of the world's oxygen.",
        "The human heart beats about 100,000 times a day.",
        "The Mona Lisa was painted by Leonardo da Vinci.",
        "The Great Barrier Reef is the world's largest coral reef system.",
        "The Moon is Earth's only natural satellite.",
        "Albert Einstein developed the theory of relativity.",
        "The Pacific Ocean is the largest and deepest ocean on Earth.",
        "Atoms are the basic units of matter.",
        "The Statue of Liberty was a gift from France to the United States.",
    ];

    // Distribute queries among rerank actors
    let mut rerank_futures = Vec::new();

    for (i, query) in queries.iter().enumerate() {
        let rerank_id = &rerank_actors[i % num_rerank_actors];
        let future = relay_ctx.send_and_wait_reply::<Rerank, RankTexts>(
            RankTexts { query: query.to_string(), texts: texts.iter().map(|s| s.to_string()).collect() },
            rerank_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
        );
        rerank_futures.push(future);
    }

    let all_rankings = join_all(rerank_futures).await;

    for (i, ranking_result) in all_rankings.into_iter().enumerate() {
        match ranking_result {
            Ok(ranked_texts) => {
                info!("Query {}: {}", i, queries[i]);
                for (j, ranked_text) in ranked_texts.texts.into_iter().take(3).enumerate() {
                    info!("  Top {}: {} (score: {:.4})", j + 1, texts[ranked_text.index], ranked_text.score);
                }
            }
            Err(e) => error!("Error processing query {}: {:?}", i, e),
        }
    }

    // Abort all rerank actors
    for handle in rerank_handles {
        handle.abort();
    }

    // Export the database for debugging
    dbg_export_db!(engine);

    Ok(())
}
