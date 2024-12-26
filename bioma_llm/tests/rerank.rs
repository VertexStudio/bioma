use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use test_log::test;

#[derive(thiserror::Error, Debug)]
enum TestError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Rerank error: {0}")]
    Rerank(#[from] RerankError),
}

#[test(tokio::test)]
async fn test_rerank_basic() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Spawn the rerank actor
    let rerank_id = ActorId::of::<Rerank>("/rerank");
    let (mut rerank_ctx, mut rerank_actor) =
        Actor::spawn(engine.clone(), rerank_id.clone(), Rerank::default(), SpawnOptions::default()).await?;

    let rerank_handle = tokio::spawn(async move {
        if let Err(e) = rerank_actor.start(&mut rerank_ctx).await {
            eprintln!("Rerank actor error: {}", e);
        }
    });

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Test texts
    let texts = vec![
        "Hello, how are you?".to_string(),
        "What is the weather like today?".to_string(),
        "I love programming!".to_string(),
        "The quick brown fox jumps over the lazy dog.".to_string(),
        "The capital of Japan is Tokyo.".to_string(),
        "Artificial intelligence is revolutionizing many industries.".to_string(),
        "Climate change is a global concern.".to_string(),
        "The Eiffel Tower is in Paris, France.".to_string(),
        "Quantum computing promises to solve complex problems faster.".to_string(),
        "The Great Wall of China is visible from space.".to_string(),
        "Renewable energy sources are becoming more popular.".to_string(),
        "The human brain is a complex organ.".to_string(),
        "Machine learning algorithms can predict patterns in data.".to_string(),
        "The Amazon rainforest is often called the 'lungs of the Earth'.".to_string(),
        "Blockchain technology has applications beyond cryptocurrency.".to_string(),
        "The Mona Lisa is displayed in the Louvre Museum.".to_string(),
        "Space exploration continues to push the boundaries of human knowledge.".to_string(),
        "The Internet of Things connects everyday devices to the internet.".to_string(),
        "Genetic engineering has the potential to cure inherited diseases.".to_string(),
        "Virtual reality is changing how we experience entertainment.".to_string(),
        "The Great Barrier Reef is the world's largest coral reef system.".to_string(),
        "Autonomous vehicles are becoming more common on roads.".to_string(),
        "The human genome project mapped all human genes.".to_string(),
        "5G networks promise faster and more reliable internet connections.".to_string(),
    ];

    // Query to use for reranking
    let query = "What is the weather in Tokyo?";

    // Rerank the texts
    let ranked_texts = relay_ctx
        .send_and_wait_reply::<Rerank, RankTexts>(
            RankTexts::builder().query(query.to_string()).texts(texts.clone()).build(),
            &rerank_id,
            SendOptions::default(),
        )
        .await?;

    // Check the results
    assert_eq!(ranked_texts.texts.len(), texts.len());
    assert!(ranked_texts.texts[0].score >= ranked_texts.texts[1].score);
    assert_eq!(texts[ranked_texts.texts[0].index], "What is the weather like today?");

    // Additional assertions
    assert!(ranked_texts.texts[0].score > ranked_texts.texts[ranked_texts.texts.len() - 1].score);

    // Terminate the actor
    rerank_handle.abort();

    Ok(())
}

#[test(tokio::test)]
async fn test_rerank_pool() -> Result<(), TestError> {
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
                eprintln!("Rerank actor {} error: {}", i, e);
            }
        });

        rerank_actors.push(rerank_id);
        rerank_handles.push(rerank_handle);
    }

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Texts to rerank
    let texts = vec![
        "Hello, how are you?",
        "What is the meaning of life?",
        "The quick brown fox jumps over the lazy dog",
        "Why is the sky blue?",
        "What is the capital of France?",
        "How does photosynthesis work?",
        "What are the primary colors?",
        "Explain the theory of relativity",
        "Who wrote Romeo and Juliet?",
        "What is the boiling point of water?",
        "What is the largest planet in our solar system?",
        "How do airplanes fly?",
        "What is the process of photosynthesis?",
        "Who invented the telephone?",
        "What is the chemical formula for water?",
        "How does a computer work?",
        "What are the main components of DNA?",
        "What causes earthquakes?",
        "How do vaccines work?",
        "What is the difference between weather and climate?",
        "How does the human digestive system function?",
        "What is the theory of evolution?",
        "How do solar panels generate electricity?",
        "What is the function of the ozone layer?",
        "How do nuclear power plants work?",
        "What is the process of mitosis?",
        "How do black holes form?",
        "What is the greenhouse effect?",
        "How does the human immune system work?",
        "What is the difference between renewable and non-renewable energy sources?",
    ];

    // Distribute texts among rerank actors
    let chunks: Vec<Vec<String>> = texts
        .chunks((texts.len() + num_rerank_actors - 1) / num_rerank_actors)
        .map(|chunk| chunk.iter().map(|&s| s.to_string()).collect())
        .collect();

    let mut rerank_futures = Vec::new();

    for (i, chunk) in chunks.iter().enumerate() {
        let rerank_id = &rerank_actors[i];
        let future = relay_ctx.send_and_wait_reply::<Rerank, RankTexts>(
            RankTexts::builder().query("General knowledge".to_string()).texts(chunk.clone()).build(),
            rerank_id,
            SendOptions::default(),
        );
        rerank_futures.push(future);
    }

    let all_ranked_texts = futures::future::join_all(rerank_futures).await;

    for ranked_texts_result in all_ranked_texts {
        let ranked_texts = ranked_texts_result?;
        assert!(!ranked_texts.texts.is_empty());
        for ranked_text in &ranked_texts.texts {
            assert!(ranked_text.index < texts.len());
        }
    }

    // Terminate all rerank actors
    for handle in rerank_handles {
        handle.abort();
    }

    Ok(())
}
