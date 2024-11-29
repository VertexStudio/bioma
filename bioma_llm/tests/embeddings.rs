use bioma_actor::prelude::*;
use bioma_llm::embeddings::{ImageModel, Model};
use bioma_llm::prelude::*;
use test_log::test;
use tracing::error;

#[derive(thiserror::Error, Debug)]
enum TestError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Embeddings error: {0}")]
    Embeddings(#[from] EmbeddingsError),
}

const CLIPVIT32_EMBEDDING_LENGTH: usize = 512;
const NOMIC_V15_EMBEDDING_LENGTH: usize = 768;

#[test(tokio::test)]
async fn test_embeddings_generate_nomic_v15() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Spawn the Nomic v1.5 embeddings actor
    let embeddings_nomic_id = ActorId::of::<Embeddings>("/embeddings/nomic_v15");
    let (mut embeddings_ctx, mut embeddings_actor) =
        Actor::spawn(engine.clone(), embeddings_nomic_id.clone(), Embeddings::default(), SpawnOptions::default())
            .await?;
    let embeddings_nomic_handle = tokio::spawn(async move {
        if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
            error!("Embeddings actor error: {}", e);
        }
    });

    // Spawn a relay actor for the Nomic v1.5 embeddings
    let nomic_relay_id = ActorId::of::<Relay>("/relay/nomic_v15");
    let (nomic_relay_ctx, _nomic_relay_actor) =
        Actor::spawn(engine.clone(), nomic_relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Test texts
    let texts = vec!["Hello, world!", "This is a test."];

    // Generate embeddings for the Nomic v1.5 embeddings actor
    let nomic_embeddings = nomic_relay_ctx
        .send::<Embeddings, GenerateEmbeddings>(
            GenerateEmbeddings { content: EmbeddingContent::Text(texts.iter().map(|text| text.to_string()).collect()) },
            &embeddings_nomic_id,
            SendOptions::default(),
        )
        .await?;

    // Check the results for the Nomic v1.5 embeddings
    assert_eq!(nomic_embeddings.embeddings.len(), texts.len());
    for embedding in &nomic_embeddings.embeddings {
        assert_eq!(embedding.len(), NOMIC_V15_EMBEDDING_LENGTH);
    }

    // Additional assertions for the Nomic v1.5 embeddings
    assert!(nomic_embeddings.embeddings.iter().all(|e| e.iter().all(|&val| val.is_finite())));
    assert!(nomic_embeddings.embeddings.iter().all(|e| e.iter().any(|&val| val != 0.0)));

    // Terminate the Nomic v1.5 embeddings actor
    embeddings_nomic_handle.abort();

    dbg_export_db!(engine);

    Ok(())
}

#[test(tokio::test)]
async fn test_embeddings_generate_clipvit32() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Spawn the CLIP-ViT-32 embeddings actor
    let embeddings_clipvit32_id = ActorId::of::<Embeddings>("/embeddings/clipvit32");
    let (mut embeddings_ctx, mut embeddings_actor) = Actor::spawn(
        engine.clone(),
        embeddings_clipvit32_id.clone(),
        Embeddings::builder()
            .table_name_prefix("clipvit32".to_string())
            .model(Model::ClipVitB32Text)
            .image_model(ImageModel::ClipVitB32Vision)
            .build(),
        SpawnOptions::default(),
    )
    .await?;
    let embeddings_clipvit32_handle = tokio::spawn(async move {
        if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
            error!("Embeddings actor error: {}", e);
        }
    });

    // Spawn a relay actor for the CLIP-ViT-32 embeddings
    let clipvit32_relay_id = ActorId::of::<Relay>("/relay/clipvit32");
    let (clipvit32_relay_ctx, _clipvit32_relay_actor) =
        Actor::spawn(engine.clone(), clipvit32_relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Test texts
    let texts = vec!["Hello, world!", "This is a test."];

    // Generate embeddings for the CLIP-ViT-32 embeddings actor
    let clipvit32_embeddings = clipvit32_relay_ctx
        .send::<Embeddings, GenerateEmbeddings>(
            GenerateEmbeddings { content: EmbeddingContent::Text(texts.iter().map(|text| text.to_string()).collect()) },
            &embeddings_clipvit32_id,
            SendOptions::default(),
        )
        .await?;

    // Check the results for the CLIP-ViT-32 embeddings
    assert_eq!(clipvit32_embeddings.embeddings.len(), texts.len());
    for embedding in &clipvit32_embeddings.embeddings {
        assert_eq!(embedding.len(), CLIPVIT32_EMBEDDING_LENGTH);
    }

    // Additional assertions for the CLIP-ViT-32 embeddings
    assert!(clipvit32_embeddings.embeddings.iter().all(|e| e.iter().all(|&val| val.is_finite())));
    assert!(clipvit32_embeddings.embeddings.iter().all(|e| e.iter().any(|&val| val != 0.0)));

    // Terminate the CLIP-ViT-32 embeddings actor
    embeddings_clipvit32_handle.abort();

    dbg_export_db!(engine);

    Ok(())
}

#[test(tokio::test)]
async fn test_embeddings_generate_multiple_types() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Spawn the Nomic v1.5 embeddings actor
    let embeddings_nomic_id = ActorId::of::<Embeddings>("/embeddings/nomic_v15");
    let (mut embeddings_ctx, mut embeddings_actor) =
        Actor::spawn(engine.clone(), embeddings_nomic_id.clone(), Embeddings::default(), SpawnOptions::default())
            .await?;
    let embeddings_nomic_handle = tokio::spawn(async move {
        if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
            error!("Embeddings actor error: {}", e);
        }
    });

    // Spawn the CLIP-ViT-32 embeddings actor
    let embeddings_clipvit32_id = ActorId::of::<Embeddings>("/embeddings/clipvit32");
    let (mut embeddings_ctx, mut embeddings_actor) = Actor::spawn(
        engine.clone(),
        embeddings_clipvit32_id.clone(),
        Embeddings::builder()
            .table_name_prefix("clipvit32".to_string())
            .model(Model::ClipVitB32Text)
            .image_model(ImageModel::ClipVitB32Vision)
            .build(),
        SpawnOptions::default(),
    )
    .await?;
    let embeddings_clipvit32_handle = tokio::spawn(async move {
        if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
            error!("Embeddings actor error: {}", e);
        }
    });

    // Spawn a relay actor for the Nomic v1.5 embeddings
    let nomic_relay_id = ActorId::of::<Relay>("/relay/nomic_v15");
    let (nomic_relay_ctx, _nomic_relay_actor) =
        Actor::spawn(engine.clone(), nomic_relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Spawn a relay actor for the CLIP-ViT-32 embeddings
    let clipvit32_relay_id = ActorId::of::<Relay>("/relay/clipvit32");
    let (clipvit32_relay_ctx, _clipvit32_relay_actor) =
        Actor::spawn(engine.clone(), clipvit32_relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Test texts
    let texts = vec!["Hello, world!", "This is a test."];

    // Generate embeddings for the Nomic v1.5 embeddings actor
    let nomic_embeddings = nomic_relay_ctx
        .send::<Embeddings, GenerateEmbeddings>(
            GenerateEmbeddings { content: EmbeddingContent::Text(texts.iter().map(|text| text.to_string()).collect()) },
            &embeddings_nomic_id,
            SendOptions::default(),
        )
        .await?;

    // Generate embeddings for the CLIP-ViT-32 embeddings actor
    let clipvit32_embeddings = clipvit32_relay_ctx
        .send::<Embeddings, GenerateEmbeddings>(
            GenerateEmbeddings { content: EmbeddingContent::Text(texts.iter().map(|text| text.to_string()).collect()) },
            &embeddings_clipvit32_id,
            SendOptions::default(),
        )
        .await?;

    // Check the results for the Nomic v1.5 embeddings
    assert_eq!(nomic_embeddings.embeddings.len(), texts.len());
    for embedding in &nomic_embeddings.embeddings {
        assert_eq!(embedding.len(), NOMIC_V15_EMBEDDING_LENGTH);
    }

    // Check the results for the CLIP-ViT-32 embeddings
    assert_eq!(clipvit32_embeddings.embeddings.len(), texts.len());
    for embedding in &clipvit32_embeddings.embeddings {
        assert_eq!(embedding.len(), CLIPVIT32_EMBEDDING_LENGTH);
    }

    // Additional assertions for the Nomic v1.5 embeddings
    assert!(nomic_embeddings.embeddings.iter().all(|e| e.iter().all(|&val| val.is_finite())));
    assert!(nomic_embeddings.embeddings.iter().all(|e| e.iter().any(|&val| val != 0.0)));

    // Additional assertions for the CLIP-ViT-32 embeddings
    assert!(clipvit32_embeddings.embeddings.iter().all(|e| e.iter().all(|&val| val.is_finite())));
    assert!(clipvit32_embeddings.embeddings.iter().all(|e| e.iter().any(|&val| val != 0.0)));

    // Terminate the Nomic v1.5 embeddings actor
    embeddings_nomic_handle.abort();

    // Terminate the CLIP-ViT-32 embeddings actor
    embeddings_clipvit32_handle.abort();

    dbg_export_db!(engine);

    Ok(())
}

#[test(tokio::test)]
async fn test_embeddings_top_k_similarities() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Spawn the embeddings actor
    let embeddings_id = ActorId::of::<Embeddings>("/embeddings");
    let (mut embeddings_ctx, mut embeddings_actor) =
        Actor::spawn(engine.clone(), embeddings_id.clone(), Embeddings::default(), SpawnOptions::default()).await?;

    let embeddings_handle = tokio::spawn(async move {
        if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
            error!("Embeddings actor error: {}", e);
        }
    });

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Generate embeddings for some texts
    let texts = vec![
        "Hello, how are you?",
        "What is the weather like today?",
        "I love programming!",
        "The quick brown fox jumps over the lazy dog.",
    ];

    let _ = relay_ctx
        .send::<Embeddings, StoreEmbeddings>(
            StoreEmbeddings {
                source: "test".to_string(),
                content: EmbeddingContent::Text(texts.iter().map(|text| text.to_string()).collect()),
                metadata: None,
                tag: Some("test".to_string()),
            },
            &embeddings_id,
            SendOptions::default(),
        )
        .await?;

    // Test top-k similarities
    let query = "How are you doing?";
    let top_k = embeddings::TopK {
        query: embeddings::Query::Text(query.to_string()),
        threshold: -0.5,
        k: 2,
        tag: Some("test".to_string()),
    };

    let similarities =
        relay_ctx.send::<Embeddings, embeddings::TopK>(top_k, &embeddings_id, SendOptions::default()).await?;

    // Check the results
    assert_eq!(similarities.len(), 2);
    assert!(similarities[0].similarity >= similarities[1].similarity);
    assert_eq!(similarities[0].text, Some("Hello, how are you?".to_string()));

    // Additional assertions
    assert!(similarities.iter().all(|s| s.similarity >= -1.0 && s.similarity <= 1.0));
    assert!(similarities[0].similarity > similarities[1].similarity);

    // Terminate the actor
    embeddings_handle.abort();

    Ok(())
}

#[test(tokio::test)]
async fn test_embeddings_persistence() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Spawn the embeddings actor
    let embeddings_id = ActorId::of::<Embeddings>("/embeddings");
    let (mut embeddings_ctx, mut embeddings_actor) =
        Actor::spawn(engine.clone(), embeddings_id.clone(), Embeddings::default(), SpawnOptions::default()).await?;

    let embeddings_handle = tokio::spawn(async move {
        if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
            error!("Embeddings actor error: {}", e);
        }
    });

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Generate embeddings
    let texts = vec!["Persistent embedding test"];
    let _ = relay_ctx
        .send::<Embeddings, StoreEmbeddings>(
            StoreEmbeddings {
                source: "test".to_string(),
                content: EmbeddingContent::Text(texts.iter().map(|text| text.to_string()).collect()),
                metadata: None,
                tag: Some("persistence_test".to_string()),
            },
            &embeddings_id,
            SendOptions::default(),
        )
        .await?;

    // Terminate the actor
    embeddings_handle.abort();

    // Respawn the embeddings actor
    let (mut restored_embeddings_ctx, mut restored_embeddings_actor) = Actor::spawn(
        engine.clone(),
        embeddings_id.clone(),
        Embeddings::default(),
        SpawnOptions::builder().exists(SpawnExistsOptions::Restore).build(),
    )
    .await?;

    let restored_embeddings_handle = tokio::spawn(async move {
        if let Err(e) = restored_embeddings_actor.start(&mut restored_embeddings_ctx).await {
            error!("Restored Embeddings actor error: {}", e);
        }
    });

    // Check if the previously generated embedding is still available
    let top_k = embeddings::TopK {
        query: embeddings::Query::Text("Persistent test".to_string()),
        threshold: -0.5,
        k: 1,
        tag: Some("persistence_test".to_string()),
    };

    let similarities =
        relay_ctx.send::<Embeddings, embeddings::TopK>(top_k, &embeddings_id, SendOptions::default()).await?;

    assert_eq!(similarities.len(), 1);
    assert_eq!(similarities[0].text, Some("Persistent embedding test".to_string()));

    // Additional assertion
    assert!(similarities[0].similarity > 0.5, "Expected high similarity for persistent embedding");

    // Terminate the restored actor
    restored_embeddings_handle.abort();

    Ok(())
}

#[test(tokio::test)]
async fn test_embeddings_with_metadata() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Spawn the embeddings actor
    let embeddings_id = ActorId::of::<Embeddings>("/embeddings");
    let (mut embeddings_ctx, mut embeddings_actor) =
        Actor::spawn(engine.clone(), embeddings_id.clone(), Embeddings::default(), SpawnOptions::default()).await?;

    let embeddings_handle = tokio::spawn(async move {
        if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
            error!("Embeddings actor error: {}", e);
        }
    });

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Generate embeddings with metadata
    let texts = vec!["Text with metadata"];
    let metadata = vec![serde_json::json!({"key": "value"})];
    let _ = relay_ctx
        .send::<Embeddings, StoreEmbeddings>(
            StoreEmbeddings {
                source: "test".to_string(),
                content: EmbeddingContent::Text(texts.iter().map(|text| text.to_string()).collect()),
                metadata: Some(metadata),
                tag: Some("metadata_test".to_string()),
            },
            &embeddings_id,
            SendOptions::default(),
        )
        .await?;

    // Query for the embedding
    let top_k = embeddings::TopK {
        query: embeddings::Query::Text("Text with metadata".to_string()),
        threshold: -0.5,
        k: 1,
        tag: Some("metadata_test".to_string()),
    };

    let similarities =
        relay_ctx.send::<Embeddings, embeddings::TopK>(top_k, &embeddings_id, SendOptions::default()).await?;

    assert_eq!(similarities.len(), 1);
    assert_eq!(similarities[0].text, Some("Text with metadata".to_string()));
    assert_eq!(similarities[0].metadata, Some(serde_json::json!({"key": "value"})));

    // Additional assertions
    assert!(similarities[0].similarity > 0.9, "Expected very high similarity for exact match");
    assert_eq!(similarities[0].metadata.as_ref().unwrap()["key"], "value");

    // Terminate the actor
    embeddings_handle.abort();

    Ok(())
}

#[test(tokio::test)]
async fn test_embeddings_pool() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Create multiple embeddings actors
    let num_embeddings_actors = 3;
    let mut embeddings_actors = Vec::new();
    let mut embeddings_handles = Vec::new();

    for i in 0..num_embeddings_actors {
        let embeddings_id = ActorId::of::<Embeddings>(format!("/embeddings_{}", i));
        let (mut embeddings_ctx, mut embeddings_actor) =
            Actor::spawn(engine.clone(), embeddings_id.clone(), Embeddings::default(), SpawnOptions::default()).await?;

        let embeddings_handle = tokio::spawn(async move {
            if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
                error!("Embeddings actor {} error: {}", i, e);
            }
        });

        embeddings_actors.push(embeddings_id);
        embeddings_handles.push(embeddings_handle);
    }

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Texts to embed
    let texts = vec![
        "Hello, how are you?",
        "What is the meaning of life?",
        "The quick brown fox jumps over the lazy dog",
        "Why is the sky blue?",
        "What is the capital of the moon?",
        "How are they doing?",
        "Are you ok?",
        "What's your favorite color?",
        "Can you explain quantum physics?",
        "Where do dreams come from?",
        "How does the internet work?",
        "What's the best way to learn a new language?",
        "Why do leaves change color in autumn?",
        "What's the difference between a virus and a bacteria?",
        "How do airplanes fly?",
    ];

    // Distribute texts among embeddings actors
    let chunks: Vec<Vec<String>> = texts
        .chunks((texts.len() + num_embeddings_actors - 1) / num_embeddings_actors)
        .map(|chunk| chunk.iter().map(|&s| s.to_string()).collect())
        .collect();

    let mut embedding_futures = Vec::new();

    for (i, chunk) in chunks.iter().enumerate() {
        let embeddings_id = &embeddings_actors[i];
        let future = relay_ctx.send::<Embeddings, StoreEmbeddings>(
            StoreEmbeddings {
                source: "test".to_string(),
                content: EmbeddingContent::Text(chunk.clone()),
                metadata: None,
                tag: Some(format!("test_{}", i)),
            },
            embeddings_id,
            SendOptions::default(),
        );
        embedding_futures.push(future);
    }

    let all_embeddings = futures::future::join_all(embedding_futures).await;

    for embeddings_result in all_embeddings {
        let embeddings = embeddings_result?;
        assert!(!embeddings.lengths.is_empty());
        for length in &embeddings.lengths {
            assert_eq!(*length, NOMIC_V15_EMBEDDING_LENGTH);
        }
    }

    // Get similarities from all actors
    let mut similarity_futures = Vec::new();

    for (i, embeddings_id) in embeddings_actors.iter().enumerate() {
        let top_k = embeddings::TopK {
            query: embeddings::Query::Text("Hello, how are you?".to_string()),
            threshold: -0.5,
            k: 2,
            tag: Some(format!("test_{}", i)),
        };
        let future = relay_ctx.send::<Embeddings, embeddings::TopK>(top_k, embeddings_id, SendOptions::default());
        similarity_futures.push(future);
    }

    let all_similarities = futures::future::join_all(similarity_futures).await;

    for similarities_result in all_similarities {
        let similarities = similarities_result?;
        assert!(!similarities.is_empty());
        for similarity in similarities {
            assert!(similarity.similarity >= -0.5);
            assert!(similarity.similarity <= 1.0);
        }
    }

    // Terminate all embeddings actors
    for handle in embeddings_handles {
        handle.abort();
    }

    Ok(())
}

#[test(tokio::test)]
async fn test_image_embeddings_generate() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Spawn the embeddings actor
    let embeddings_id = ActorId::of::<Embeddings>("/embeddings");
    let (mut embeddings_ctx, mut embeddings_actor) =
        Actor::spawn(engine.clone(), embeddings_id.clone(), Embeddings::default(), SpawnOptions::default()).await?;

    let embeddings_handle = tokio::spawn(async move {
        if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
            error!("Embeddings actor error: {}", e);
        }
    });

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Generate embeddings for the image
    let image_paths = vec!["../assets/images/rust-pet.png".to_string()];
    let generated = relay_ctx
        .send::<Embeddings, GenerateEmbeddings>(
            GenerateEmbeddings { content: EmbeddingContent::Image(image_paths.clone()) },
            &embeddings_id,
            SendOptions::default(),
        )
        .await?;

    // Check the results
    assert_eq!(generated.embeddings.len(), 1);
    assert_eq!(generated.embeddings[0].len(), NOMIC_V15_EMBEDDING_LENGTH);
    assert!(generated.embeddings[0].iter().all(|&val| val.is_finite()));
    assert!(generated.embeddings[0].iter().any(|&val| val != 0.0));

    // Terminate the actor
    embeddings_handle.abort();

    Ok(())
}

#[test(tokio::test)]
async fn test_image_embeddings_store_and_search() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Spawn the embeddings actor
    let embeddings_id = ActorId::of::<Embeddings>("/embeddings");
    let (mut embeddings_ctx, mut embeddings_actor) =
        Actor::spawn(engine.clone(), embeddings_id.clone(), Embeddings::default(), SpawnOptions::default()).await?;

    let embeddings_handle = tokio::spawn(async move {
        if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
            error!("Embeddings actor error: {}", e);
        }
    });

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Store image embeddings with metadata
    let image_paths = vec!["../assets/images/elephant.jpg".to_string()];
    let metadata = vec![serde_json::json!({
        "description": "An elephant image",
        "type": "wildlife"
    })];

    let stored = relay_ctx
        .send::<Embeddings, StoreEmbeddings>(
            StoreEmbeddings {
                source: "test".to_string(),
                content: EmbeddingContent::Image(image_paths.clone()),
                metadata: Some(metadata),
                tag: Some("test_images".to_string()),
            },
            &embeddings_id,
            SendOptions::default(),
        )
        .await?;

    assert_eq!(stored.lengths.len(), 1);
    assert_eq!(stored.lengths[0], NOMIC_V15_EMBEDDING_LENGTH);

    // Search using the same image
    let top_k = embeddings::TopK {
        query: embeddings::Query::Image("../assets/images/elephant.jpg".to_string()),
        threshold: 0.5,
        k: 1,
        tag: Some("test_images".to_string()),
    };

    let similarities =
        relay_ctx.send::<Embeddings, embeddings::TopK>(top_k, &embeddings_id, SendOptions::default()).await?;

    // Check search results
    assert_eq!(similarities.len(), 1);
    assert!(similarities[0].similarity == 1.0, "Expected similarity of 1.0 for exact match");
    assert!(similarities[0].metadata.is_some());
    assert_eq!(similarities[0].metadata.as_ref().unwrap()["description"], "An elephant image");

    // Terminate the actor
    embeddings_handle.abort();

    Ok(())
}

#[test(tokio::test)]
async fn test_cross_modal_search() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Spawn the embeddings actor with CLIP model for cross-modal search
    let embeddings_id = ActorId::of::<Embeddings>("/embeddings/clip");
    let (mut embeddings_ctx, mut embeddings_actor) = Actor::spawn(
        engine.clone(),
        embeddings_id.clone(),
        Embeddings::builder().model(Model::ClipVitB32Text).image_model(ImageModel::ClipVitB32Vision).build(),
        SpawnOptions::default(),
    )
    .await?;

    let embeddings_handle = tokio::spawn(async move {
        if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
            error!("Embeddings actor error: {}", e);
        }
    });

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Store image embeddings
    let image_paths = vec!["../assets/images/elephant.jpg".to_string()];
    let _ = relay_ctx
        .send::<Embeddings, StoreEmbeddings>(
            StoreEmbeddings {
                source: "test".to_string(),
                content: EmbeddingContent::Image(image_paths),
                metadata: None,
                tag: Some("cross_modal".to_string()),
            },
            &embeddings_id,
            SendOptions::default(),
        )
        .await?;

    // Search using text query
    let top_k = embeddings::TopK {
        query: embeddings::Query::Text("an elephant".to_string()),
        threshold: 0.2,
        k: 1,
        tag: Some("cross_modal".to_string()),
    };

    let similarities =
        relay_ctx.send::<Embeddings, embeddings::TopK>(top_k, &embeddings_id, SendOptions::default()).await?;

    // Check if text query found the image
    assert_eq!(similarities.len(), 1);
    assert!(similarities[0].similarity > 0.2, "Expected reasonable similarity for cross-modal search");

    // Terminate the actor
    embeddings_handle.abort();

    Ok(())
}

#[test(tokio::test)]
async fn test_multiple_images_batch() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Spawn the embeddings actor
    let embeddings_id = ActorId::of::<Embeddings>("/embeddings");
    let (mut embeddings_ctx, mut embeddings_actor) =
        Actor::spawn(engine.clone(), embeddings_id.clone(), Embeddings::default(), SpawnOptions::default()).await?;

    let embeddings_handle = tokio::spawn(async move {
        if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
            error!("Embeddings actor error: {}", e);
        }
    });

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Use multiple images
    let image_paths = vec!["../assets/images/elephant.jpg".to_string(), "../assets/images/rust-pet.png".to_string()];

    let generated = relay_ctx
        .send::<Embeddings, GenerateEmbeddings>(
            GenerateEmbeddings { content: EmbeddingContent::Image(image_paths) },
            &embeddings_id,
            SendOptions::default(),
        )
        .await?;

    // Verify batch processing
    assert_eq!(generated.embeddings.len(), 2);
    assert!(generated.embeddings.iter().all(|emb| emb.len() == NOMIC_V15_EMBEDDING_LENGTH));
    assert!(generated.embeddings.iter().all(|emb| emb.iter().all(|&val| val.is_finite())));
    assert!(generated.embeddings.iter().all(|emb| emb.iter().any(|&val| val != 0.0)));

    embeddings_handle.abort();
    Ok(())
}

#[test(tokio::test)]
async fn test_mixed_modal_storage() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    let embeddings_id = ActorId::of::<Embeddings>("/embeddings/clip");
    let (mut embeddings_ctx, mut embeddings_actor) = Actor::spawn(
        engine.clone(),
        embeddings_id.clone(),
        Embeddings::builder().model(Model::ClipVitB32Text).image_model(ImageModel::ClipVitB32Vision).build(),
        SpawnOptions::default(),
    )
    .await?;

    let embeddings_handle = tokio::spawn(async move {
        if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
            error!("Embeddings actor error: {}", e);
        }
    });

    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Store both images and text
    let _ = relay_ctx
        .send::<Embeddings, StoreEmbeddings>(
            StoreEmbeddings {
                source: "test".to_string(),
                content: EmbeddingContent::Image(vec!["../assets/images/elephant.jpg".to_string()]),
                tag: Some("mixed".to_string()),
                metadata: Some(vec![serde_json::json!({"type": "image"})]),
            },
            &embeddings_id,
            SendOptions::default(),
        )
        .await?;

    let _ = relay_ctx
        .send::<Embeddings, StoreEmbeddings>(
            StoreEmbeddings {
                source: "test".to_string(),
                content: EmbeddingContent::Text(vec!["an elephant in the wild".to_string()]),
                tag: Some("mixed".to_string()),
                metadata: Some(vec![serde_json::json!({"type": "text"})]),
            },
            &embeddings_id,
            SendOptions::default(),
        )
        .await?;

    // Search across both modalities
    let similarities = relay_ctx
        .send::<Embeddings, embeddings::TopK>(
            embeddings::TopK {
                query: embeddings::Query::Text("elephant".to_string()),
                threshold: 0.2,
                k: 2,
                tag: Some("mixed".to_string()),
            },
            &embeddings_id,
            SendOptions::default(),
        )
        .await?;

    assert_eq!(similarities.len(), 2);
    assert!(similarities.iter().any(|s| s.metadata.as_ref().unwrap()["type"] == "image"));
    assert!(similarities.iter().any(|s| s.metadata.as_ref().unwrap()["type"] == "text"));
    assert!(similarities.iter().all(|s| s.similarity > 0.2));

    embeddings_handle.abort();
    Ok(())
}
