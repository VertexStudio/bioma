use base64::Engine as _;
use bioma_actor::prelude::Engine as ActorEngine;
use bioma_actor::prelude::*;
use bioma_llm::{
    indexer::{GlobsContent, ImagesContent, TextsContent},
    prelude::*,
    retriever::{ListSources, ListUniqueSources},
};
use serde_json;
use std::fs;
use tempfile;
use test_log::test;
use tracing::error;

#[derive(thiserror::Error, Debug)]
enum TestError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Indexer error: {0}")]
    Indexer(#[from] IndexerError),
    #[error("Retriever error: {0}")]
    Retriever(#[from] RetrieverError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[test(tokio::test)]
async fn test_indexer_basic_text() -> Result<(), TestError> {
    let engine = ActorEngine::test().await?;
    let temp_dir = tempfile::tempdir()?;

    // Create test files
    let test_files = vec![
        ("test1.txt", "This is a test file with some content."),
        ("test2.md", "# Test Markdown\nThis is a markdown file."),
        ("test3.rs", "fn main() {\n    println!(\"Hello, World!\");\n}"),
    ];

    for (filename, content) in test_files.iter() {
        let file_path = temp_dir.path().join(filename);
        fs::write(&file_path, content)?;
    }

    // Spawn the indexer actor
    let indexer_id = ActorId::of::<Indexer>("/indexer");
    let (mut indexer_ctx, mut indexer_actor) =
        Actor::spawn(engine.clone(), indexer_id.clone(), Indexer::default(), SpawnOptions::default()).await?;

    let indexer_handle = tokio::spawn(async move {
        if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
            error!("Indexer actor error: {}", e);
        }
    });

    // Spawn the retriever actor
    let retriever_id = ActorId::of::<Retriever>("/retriever");
    let (mut retriever_ctx, mut retriever_actor) =
        Actor::spawn(engine.clone(), retriever_id.clone(), Retriever::default(), SpawnOptions::default()).await?;

    let retriever_handle = tokio::spawn(async move {
        if let Err(e) = retriever_actor.start(&mut retriever_ctx).await {
            error!("Retriever actor error: {}", e);
        }
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Index the files
    let globs = vec![
        temp_dir.path().join("*.txt").to_string_lossy().into_owned(),
        temp_dir.path().join("*.md").to_string_lossy().into_owned(),
        temp_dir.path().join("*.rs").to_string_lossy().into_owned(),
    ];

    let index_result = relay_ctx
        .send_and_wait_reply::<Indexer, Index>(
            Index::builder()
                .content(IndexContent::Globs(GlobsContent { globs: globs.clone(), config: TextChunkConfig::default() }))
                .build(),
            &indexer_id,
            SendOptions::default(),
        )
        .await?;

    // Verify indexing results
    assert_eq!(index_result.indexed, 3, "Expected 3 files to be indexed");
    assert_eq!(index_result.cached, 0, "Expected no cached files");

    // List sources to verify
    let sources = relay_ctx
        .send_and_wait_reply::<Retriever, ListSources>(ListSources, &retriever_id, SendOptions::default())
        .await?;

    assert_eq!(sources.sources.len(), 3, "Expected 3 sources");
    assert!(sources.sources.iter().any(|s| s.uri.contains("test1.txt")));
    assert!(sources.sources.iter().any(|s| s.uri.contains("test2.md")));
    assert!(sources.sources.iter().any(|s| s.uri.contains("test3.rs")));

    // Test caching behavior by reindexing
    let reindex_result = relay_ctx
        .send_and_wait_reply::<Indexer, Index>(
            Index::builder()
                .content(IndexContent::Globs(GlobsContent { globs: globs.clone(), config: TextChunkConfig::default() }))
                .build(),
            &indexer_id,
            SendOptions::default(),
        )
        .await?;

    assert_eq!(reindex_result.indexed, 0, "Expected no new files to be indexed");
    assert_eq!(reindex_result.cached, 3, "Expected 3 cached files");

    // Cleanup
    indexer_handle.abort();
    retriever_handle.abort();
    temp_dir.close()?;

    Ok(())
}

#[test(tokio::test)]
async fn test_indexer_chunking() -> Result<(), TestError> {
    let engine = ActorEngine::test().await?;
    let temp_dir = tempfile::tempdir()?;

    // Create a large test file that will be chunked
    let large_content = "This is the first paragraph.\n\n".repeat(100);
    let file_path = temp_dir.path().join("large.md");
    fs::write(&file_path, large_content)?;

    // Spawn the indexer actor
    let indexer_id = ActorId::of::<Indexer>("/indexer");
    let (mut indexer_ctx, mut indexer_actor) =
        Actor::spawn(engine.clone(), indexer_id.clone(), Indexer::default(), SpawnOptions::default()).await?;

    let indexer_handle = tokio::spawn(async move {
        if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
            error!("Indexer actor error: {}", e);
        }
    });

    // Spawn the retriever actor
    let retriever_id = ActorId::of::<Retriever>("/retriever");
    let (mut retriever_ctx, mut retriever_actor) =
        Actor::spawn(engine.clone(), retriever_id.clone(), Retriever::default(), SpawnOptions::default()).await?;

    let retriever_handle = tokio::spawn(async move {
        if let Err(e) = retriever_actor.start(&mut retriever_ctx).await {
            error!("Retriever actor error: {}", e);
        }
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Index with custom chunking parameters
    let globs = vec![temp_dir.path().join("*.md").to_string_lossy().into_owned()];
    let chunk_config = TextChunkConfig {
        chunk_capacity: 100..200, // Small chunks for testing
        chunk_overlap: 50,
        chunk_batch_size: 10,
    };

    let index_result = relay_ctx
        .send_and_wait_reply::<Indexer, Index>(
            Index::builder()
                .content(IndexContent::Globs(GlobsContent::builder().globs(globs).config(chunk_config).build()))
                .build(),
            &indexer_id,
            SendOptions::default(),
        )
        .await?;

    assert_eq!(index_result.indexed, 1, "Expected 1 file to be indexed");

    // List sources to verify
    let sources = relay_ctx
        .send_and_wait_reply::<Retriever, ListSources>(ListSources, &retriever_id, SendOptions::default())
        .await?;

    assert_eq!(sources.sources.len(), 1, "Expected 1 source");
    assert!(sources.sources[0].uri.contains("large.md"));

    // Cleanup
    indexer_handle.abort();
    retriever_handle.abort();
    temp_dir.close()?;

    Ok(())
}

#[test(tokio::test)]
async fn test_indexer_delete_source() -> Result<(), TestError> {
    let engine = ActorEngine::test().await?;
    let temp_dir = tempfile::tempdir()?;

    // Create test files
    let test_files =
        vec![("delete1.txt", "This is the first test file."), ("delete2.txt", "This is the second test file.")];

    for (filename, content) in test_files.iter() {
        let file_path = temp_dir.path().join(filename);
        fs::write(&file_path, content)?;
    }

    // Spawn the indexer actor
    let indexer_id = ActorId::of::<Indexer>("/indexer");
    let (mut indexer_ctx, mut indexer_actor) =
        Actor::spawn(engine.clone(), indexer_id.clone(), Indexer::default(), SpawnOptions::default()).await?;

    let indexer_handle = tokio::spawn(async move {
        if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
            error!("Indexer actor error: {}", e);
        }
    });

    // Spawn the retriever actor
    let retriever_id = ActorId::of::<Retriever>("/retriever");
    let (mut retriever_ctx, mut retriever_actor) =
        Actor::spawn(engine.clone(), retriever_id.clone(), Retriever::default(), SpawnOptions::default()).await?;

    let retriever_handle = tokio::spawn(async move {
        if let Err(e) = retriever_actor.start(&mut retriever_ctx).await {
            error!("Retriever actor error: {}", e);
        }
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Index the files
    let glob_path = temp_dir.path().join("*.txt").to_string_lossy().into_owned();
    let index_result = relay_ctx
        .send_and_wait_reply::<Indexer, Index>(
            Index::builder()
                .content(IndexContent::Globs(GlobsContent {
                    globs: vec![glob_path.clone()],
                    config: TextChunkConfig::default(),
                }))
                .build(),
            &indexer_id,
            SendOptions::default(),
        )
        .await?;

    assert_eq!(index_result.indexed, 2, "Expected 2 files to be indexed");

    // List initial sources
    let initial_sources = relay_ctx
        .send_and_wait_reply::<Retriever, ListSources>(ListSources, &retriever_id, SendOptions::default())
        .await?;

    assert_eq!(initial_sources.sources.len(), 2, "Expected 2 initial sources");

    // Delete using the glob path
    let delete_result = relay_ctx
        .send_and_wait_reply::<Indexer, DeleteSource>(
            DeleteSource { sources: vec!["/global".to_string()], delete_from_disk: true },
            &indexer_id,
            SendOptions::default(),
        )
        .await?;

    assert_eq!(delete_result.deleted_sources.len(), 2, "Expected 2 sources to be deleted");
    assert!(delete_result.deleted_embeddings > 0, "Expected some embeddings to be deleted");

    // Verify remaining sources
    let remaining_sources = relay_ctx
        .send_and_wait_reply::<Retriever, ListSources>(ListSources, &retriever_id, SendOptions::default())
        .await?;

    assert_eq!(remaining_sources.sources.len(), 0, "Expected no remaining sources");

    // Cleanup
    indexer_handle.abort();
    retriever_handle.abort();
    temp_dir.close()?;

    Ok(())
}

#[test(tokio::test)]
async fn test_retriever_list_unique_sources() -> Result<(), TestError> {
    let engine = ActorEngine::test().await?;
    let temp_dir = tempfile::tempdir()?;

    // Create test files
    let test_files = vec![
        ("source1.txt", "This is content for source 1."),
        ("source2.txt", "This is content for source 2."),
        ("source3a.txt", "This is the first document for source 3."),
        ("source3b.txt", "This is the second document for source 3."),
    ];

    for (filename, content) in test_files.iter() {
        let file_path = temp_dir.path().join(filename);
        fs::write(&file_path, content)?;
    }

    // Spawn the indexer actor
    let indexer_id = ActorId::of::<Indexer>("/indexer");
    let (mut indexer_ctx, mut indexer_actor) =
        Actor::spawn(engine.clone(), indexer_id.clone(), Indexer::default(), SpawnOptions::default()).await?;

    let indexer_handle = tokio::spawn(async move {
        if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
            error!("Indexer actor error: {}", e);
        }
    });

    // Spawn the retriever actor
    let retriever_id = ActorId::of::<Retriever>("/retriever");
    let (mut retriever_ctx, mut retriever_actor) =
        Actor::spawn(engine.clone(), retriever_id.clone(), Retriever::default(), SpawnOptions::default()).await?;

    let retriever_handle = tokio::spawn(async move {
        if let Err(e) = retriever_actor.start(&mut retriever_ctx).await {
            error!("Retriever actor error: {}", e);
        }
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Index files with different sources
    // Source 1
    let source1_path = temp_dir.path().join("source1.txt").to_string_lossy().into_owned();
    let source1 = "/test/source1".to_string();
    let index_result1 = relay_ctx
        .send_and_wait_reply::<Indexer, Index>(
            Index::builder()
                .content(IndexContent::Globs(GlobsContent {
                    globs: vec![source1_path],
                    config: TextChunkConfig::default(),
                }))
                .source(source1.clone())
                .build(),
            &indexer_id,
            SendOptions::default(),
        )
        .await?;
    assert_eq!(index_result1.indexed, 1, "Expected 1 file to be indexed for source1");

    // Source 2
    let source2_path = temp_dir.path().join("source2.txt").to_string_lossy().into_owned();
    let source2 = "/test/source2".to_string();
    let index_result2 = relay_ctx
        .send_and_wait_reply::<Indexer, Index>(
            Index::builder()
                .content(IndexContent::Globs(GlobsContent {
                    globs: vec![source2_path],
                    config: TextChunkConfig::default(),
                }))
                .source(source2.clone())
                .build(),
            &indexer_id,
            SendOptions::default(),
        )
        .await?;
    assert_eq!(index_result2.indexed, 1, "Expected 1 file to be indexed for source2");

    // Source 3 (two documents)
    let source3_paths = vec![
        temp_dir.path().join("source3a.txt").to_string_lossy().into_owned(),
        temp_dir.path().join("source3b.txt").to_string_lossy().into_owned(),
    ];
    let source3 = "/test/source3".to_string();
    let index_result3 = relay_ctx
        .send_and_wait_reply::<Indexer, Index>(
            Index::builder()
                .content(IndexContent::Globs(GlobsContent { globs: source3_paths, config: TextChunkConfig::default() }))
                .source(source3.clone())
                .build(),
            &indexer_id,
            SendOptions::default(),
        )
        .await?;
    assert_eq!(index_result3.indexed, 2, "Expected 2 files to be indexed for source3");

    // Test ListUniqueSources
    let unique_sources: retriever::ListedUniqueSources = relay_ctx
        .send_and_wait_reply::<Retriever, ListUniqueSources>(ListUniqueSources, &retriever_id, SendOptions::default())
        .await?;

    assert_eq!(unique_sources.sources.len(), 3, "Expected 3 unique sources");

    // Validate document count for each source
    for source_info in &unique_sources.sources {
        if source_info.source == source1 {
            assert_eq!(source_info.count, 1, "Source1 should have 1 document");
        } else if source_info.source == source2 {
            assert_eq!(source_info.count, 1, "Source2 should have 1 document");
        } else if source_info.source == source3 {
            assert_eq!(source_info.count, 2, "Source3 should have 2 documents");
        }
    }

    // Test ListSources to verify total document count
    let all_sources = relay_ctx
        .send_and_wait_reply::<Retriever, ListSources>(ListSources, &retriever_id, SendOptions::default())
        .await?;

    assert_eq!(all_sources.sources.len(), 4, "Expected 4 total documents across all sources");

    // Cleanup
    indexer_handle.abort();
    retriever_handle.abort();
    temp_dir.close()?;

    Ok(())
}

#[test(tokio::test)]
async fn test_indexer_with_summary() -> Result<(), TestError> {
    let engine = ActorEngine::test().await?;
    let temp_dir = tempfile::tempdir()?;

    // Create a test file with content that warrants summarization
    let content = r#"# Important Document
    
This is a detailed document about a complex topic.
It contains multiple paragraphs of information.

## First Section
The first section discusses key concepts and ideas.
These concepts are fundamental to understanding the topic.

## Second Section
The second section builds upon the first section.
It provides practical examples and use cases.
"#;
    let file_path = temp_dir.path().join("document.md");
    fs::write(&file_path, content)?;

    // Spawn the indexer actor
    let indexer_id = ActorId::of::<Indexer>("/indexer");
    let (mut indexer_ctx, mut indexer_actor) =
        Actor::spawn(engine.clone(), indexer_id.clone(), Indexer::default(), SpawnOptions::default()).await?;

    let indexer_handle = tokio::spawn(async move {
        if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
            error!("Indexer actor error: {}", e);
        }
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    let source = "/test/indexer/with/summary".to_string();

    // Index with summary generation enabled
    let globs = vec![temp_dir.path().join("*.md").to_string_lossy().into_owned()];
    let index_result = relay_ctx
        .send_and_wait_reply::<Indexer, Index>(
            Index::builder()
                .content(IndexContent::Globs(GlobsContent { globs: globs.clone(), config: TextChunkConfig::default() }))
                .summarize(true)
                .source(source.clone())
                .build(),
            &indexer_id,
            SendOptions::default(),
        )
        .await?;

    assert_eq!(index_result.indexed, 1, "Expected 1 file to be indexed");

    // Verify summary was stored in the database
    let db = engine.db();
    let query = "SELECT VALUE summary FROM source WHERE id.source = $source";
    let mut results =
        db.lock().await.query(query).bind(("source", source.clone())).await.map_err(SystemActorError::from)?;

    let summary: Option<String> =
        results.take::<Vec<Option<String>>>(0).map_err(SystemActorError::from)?.pop().flatten();
    assert!(summary.is_some(), "Summary should be stored in the database");
    let summary = summary.unwrap();
    assert!(summary.contains("**URI**:"), "Summary should contain URI");
    assert!(summary.contains("**Summary**:"), "Summary should contain summary section");

    // Cleanup
    indexer_handle.abort();
    temp_dir.close()?;

    Ok(())
}

#[test(tokio::test)]
async fn test_indexer_without_summary() -> Result<(), TestError> {
    let engine = ActorEngine::test().await?;
    let temp_dir = tempfile::tempdir()?;

    // Create a test file
    let content = "This is a test document that should not be summarized.";
    let file_path = temp_dir.path().join("no_summary.md");
    fs::write(&file_path, content)?;

    // Spawn the indexer actor
    let indexer_id = ActorId::of::<Indexer>("/indexer");
    let (mut indexer_ctx, mut indexer_actor) =
        Actor::spawn(engine.clone(), indexer_id.clone(), Indexer::default(), SpawnOptions::default()).await?;

    let indexer_handle = tokio::spawn(async move {
        if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
            error!("Indexer actor error: {}", e);
        }
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    let source = "/test/indexer/without/summary".to_string();

    // Index with summary generation disabled
    let globs = vec![temp_dir.path().join("*.md").to_string_lossy().into_owned()];
    let index_result = relay_ctx
        .send_and_wait_reply::<Indexer, Index>(
            Index::builder()
                .content(IndexContent::Globs(GlobsContent { globs: globs.clone(), config: TextChunkConfig::default() }))
                .summarize(false)
                .source(source.clone())
                .build(),
            &indexer_id,
            SendOptions::default(),
        )
        .await?;

    assert_eq!(index_result.indexed, 1, "Expected 1 file to be indexed");

    // Verify no summary was stored in the database
    let db = engine.db();
    let query = "SELECT VALUE summary FROM source WHERE id.source = $source";
    let mut results =
        db.lock().await.query(query).bind(("source", source.clone())).await.map_err(SystemActorError::from)?;

    let summary: Option<String> =
        results.take::<Vec<Option<String>>>(0).map_err(SystemActorError::from)?.pop().flatten();
    assert!(summary.is_none(), "Summary should not be stored when summarize is false");

    // Cleanup
    indexer_handle.abort();
    temp_dir.close()?;

    Ok(())
}

#[test(tokio::test)]
async fn test_indexer_with_images_summary() -> Result<(), TestError> {
    let engine = ActorEngine::test().await?;
    let temp_dir = tempfile::tempdir()?;

    // Copy test images to temp directory
    let test_images =
        vec![("../assets/images/elephant.jpg", "elephant.jpg"), ("../assets/images/rust-pet.png", "rust-pet.png")];

    for (src, dest) in test_images.iter() {
        let dest_path = temp_dir.path().join(dest);
        fs::copy(src, &dest_path)?;
    }

    // Spawn the indexer actor with explicit summary configuration
    let mut indexer = Indexer::default();
    indexer.summary =
        Summary::builder().chat(Chat::builder().model(std::borrow::Cow::Borrowed("llama3.2:3b")).build()).build();

    let indexer_id = ActorId::of::<Indexer>("/indexer");
    let (mut indexer_ctx, mut indexer_actor) =
        Actor::spawn(engine.clone(), indexer_id.clone(), indexer, SpawnOptions::default()).await?;

    let indexer_handle = tokio::spawn(async move {
        if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
            error!("Indexer actor error: {}", e);
        }
    });

    // Give actors time to initialize
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    let source = "/test/indexer/with/images/summary".to_string();

    // Index with summary generation enabled
    let globs = vec![
        temp_dir.path().join("*.jpg").to_string_lossy().into_owned(),
        temp_dir.path().join("*.png").to_string_lossy().into_owned(),
    ];

    let index_result = relay_ctx
        .send_and_wait_reply::<Indexer, Index>(
            Index::builder()
                .content(IndexContent::Globs(GlobsContent { globs: globs.clone(), config: TextChunkConfig::default() }))
                .summarize(true)
                .source(source.clone())
                .build(),
            &indexer_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(30)).build(),
        )
        .await?;

    assert_eq!(index_result.indexed, 2, "Expected 2 images to be indexed");
    assert_eq!(index_result.cached, 0, "Expected no cached images");

    // Verify summaries were stored in the database
    let db = engine.db();
    let query = "SELECT VALUE summary FROM source WHERE id.source = $source";
    let mut results = db.lock().await.query(query).bind(("source", source)).await.map_err(SystemActorError::from)?;

    let summaries: Vec<Option<String>> = results.take(0).map_err(SystemActorError::from)?;
    assert!(!summaries.is_empty(), "Should find at least one image summary");

    // Count how many actual summaries we have (not None)
    let valid_summaries: Vec<_> = summaries.into_iter().flatten().collect();
    assert_eq!(valid_summaries.len(), 2, "Expected summaries for both images");

    for summary in valid_summaries {
        assert!(summary.contains("**URI**:"), "Summary should contain URI");
        assert!(summary.contains("**Summary**:"), "Summary should contain summary section");
    }

    // Cleanup
    indexer_handle.abort();
    temp_dir.close()?;

    Ok(())
}

#[test(tokio::test)]
async fn test_indexer_direct_texts() -> Result<(), TestError> {
    let engine = ActorEngine::test().await?;

    // Spawn the indexer actor
    let indexer_id = ActorId::of::<Indexer>("/indexer");
    let (mut indexer_ctx, mut indexer_actor) =
        Actor::spawn(engine.clone(), indexer_id.clone(), Indexer::default(), SpawnOptions::default()).await?;

    let indexer_handle = tokio::spawn(async move {
        if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
            error!("Indexer actor error: {}", e);
        }
    });

    // Spawn the retriever actor
    let retriever_id = ActorId::of::<Retriever>("/retriever");
    let (mut retriever_ctx, mut retriever_actor) =
        Actor::spawn(engine.clone(), retriever_id.clone(), Retriever::default(), SpawnOptions::default()).await?;

    let retriever_handle = tokio::spawn(async move {
        if let Err(e) = retriever_actor.start(&mut retriever_ctx).await {
            error!("Retriever actor error: {}", e);
        }
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    let source = "/test/indexer/direct/texts".to_string();

    // Create test texts with different content types
    let texts: Vec<String> = vec![
        "# Markdown Content\n\nThis is a test markdown document.".to_string(),
        "fn main() {\n    println!(\"Hello from Rust!\");\n}".to_string(),
        "This is plain text content for testing.".to_string(),
    ];

    // Index with custom chunk configuration
    let chunk_config = TextChunkConfig {
        chunk_capacity: 100..200, // Small chunks for testing
        chunk_overlap: 50,
        chunk_batch_size: 10,
    };

    let index_result = relay_ctx
        .send_and_wait_reply::<Indexer, Index>(
            Index::builder()
                .content(IndexContent::Texts(TextsContent::builder().texts(texts).config(chunk_config).build()))
                .source(source.clone())
                .build(),
            &indexer_id,
            SendOptions::default(),
        )
        .await?;

    assert_eq!(index_result.indexed, 3, "Expected 3 texts to be indexed");
    assert_eq!(index_result.cached, 0, "Expected no cached texts");

    // List sources to verify
    let sources = relay_ctx
        .send_and_wait_reply::<Retriever, ListSources>(ListSources, &retriever_id, SendOptions::default())
        .await?;

    assert_eq!(sources.sources.len(), 3, "Expected 3 sources");

    // Cleanup
    indexer_handle.abort();
    retriever_handle.abort();

    Ok(())
}

#[test(tokio::test)]
async fn test_indexer_direct_images() -> Result<(), TestError> {
    let engine = ActorEngine::test().await?;

    // Spawn the indexer actor
    let indexer_id = ActorId::of::<Indexer>("/indexer");
    let (mut indexer_ctx, mut indexer_actor) =
        Actor::spawn(engine.clone(), indexer_id.clone(), Indexer::default(), SpawnOptions::default()).await?;

    let indexer_handle = tokio::spawn(async move {
        if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
            error!("Indexer actor error: {}", e);
        }
    });

    // Give actors time to initialize
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    let source = "/test/indexer/direct/images".to_string();

    // Read test images and convert to base64
    let test_images = vec!["../assets/images/elephant.jpg".to_string(), "../assets/images/rust-pet.png".to_string()];
    let base64_images: Vec<String> = test_images
        .iter()
        .map(|path| {
            let image_data = fs::read(path).unwrap();
            base64::engine::general_purpose::STANDARD.encode(&image_data)
        })
        .collect();

    let index_result = relay_ctx
        .send_and_wait_reply::<Indexer, Index>(
            Index::builder()
                .content(IndexContent::Images(ImagesContent::builder().images(base64_images).build()))
                .source(source.clone())
                .build(),
            &indexer_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(30)).build(),
        )
        .await?;

    assert_eq!(index_result.indexed, 2, "Expected 2 images to be indexed");
    assert_eq!(index_result.cached, 0, "Expected no cached images");

    // Cleanup
    indexer_handle.abort();

    Ok(())
}

#[test]
fn test_index_json_structure() -> Result<(), TestError> {
    // Raw JSON strings representing each content type
    let raw_globs_json = r#"{
        "globs": ["*.txt", "*.md"],
        "chunk_capacity": {"start": 500, "end": 2000},
        "chunk_overlap": 200,
        "chunk_batch_size": 50,
        "source": "/test/source",
        "summarize": true
    }"#;

    let raw_texts_json = r#"{
        "texts": ["Test content 1", "Test content 2"],
        "mime_type": "text/markdown",
        "chunk_capacity": {"start": 500, "end": 2000},
        "chunk_overlap": 200,
        "chunk_batch_size": 50,
        "source": "/test/source",
        "summarize": false
    }"#;

    let raw_images_json = r#"{
        "images": ["base64_image_data"],
        "mime_type": "image/jpeg",
        "source": "/test/source",
        "summarize": true
    }"#;

    // Deserialize raw JSON into structs
    let globs_from_json: Index = serde_json::from_str(raw_globs_json)?;
    let texts_from_json: Index = serde_json::from_str(raw_texts_json)?;
    let images_from_json: Index = serde_json::from_str(raw_images_json)?;

    // Create equivalent structs using builders
    let globs_from_builder = Index::builder()
        .content(IndexContent::Globs(GlobsContent {
            globs: vec!["*.txt".to_string(), "*.md".to_string()],
            config: TextChunkConfig { chunk_capacity: 500..2000, chunk_overlap: 200, chunk_batch_size: 50 },
        }))
        .source("/test/source".to_string())
        .summarize(true)
        .build();

    let texts_from_builder = Index::builder()
        .content(IndexContent::Texts(TextsContent {
            texts: vec!["Test content 1".to_string(), "Test content 2".to_string()],
            mime_type: "text/markdown".to_string(),
            config: TextChunkConfig { chunk_capacity: 500..2000, chunk_overlap: 200, chunk_batch_size: 50 },
        }))
        .source("/test/source".to_string())
        .summarize(false)
        .build();

    let images_from_builder = Index::builder()
        .content(IndexContent::Images(ImagesContent {
            images: vec!["base64_image_data".to_string()],
            mime_type: Some("image/jpeg".to_string()),
        }))
        .source("/test/source".to_string())
        .summarize(true)
        .build();

    // Compare deserialized JSON with builder-created structs
    assert_eq!(
        serde_json::to_string(&globs_from_json)?,
        serde_json::to_string(&globs_from_builder)?,
        "Globs JSON structure should match builder-created struct"
    );

    assert_eq!(
        serde_json::to_string(&texts_from_json)?,
        serde_json::to_string(&texts_from_builder)?,
        "Texts JSON structure should match builder-created struct"
    );

    assert_eq!(
        serde_json::to_string(&images_from_json)?,
        serde_json::to_string(&images_from_builder)?,
        "Images JSON structure should match builder-created struct"
    );

    // Verify JSON structure details
    let globs_value: serde_json::Value = serde_json::from_str(raw_globs_json)?;
    assert!(globs_value["globs"].is_array(), "Globs should be an array");
    assert!(globs_value["chunk_capacity"].is_object(), "chunk_capacity should be an object");
    assert!(globs_value["chunk_overlap"].is_number(), "chunk_overlap should be a number");
    assert!(globs_value["chunk_batch_size"].is_number(), "chunk_batch_size should be a number");
    assert_eq!(globs_value["source"], "/test/source", "Source should match");
    assert_eq!(globs_value["summarize"], true, "Summarize should match");

    let texts_value: serde_json::Value = serde_json::from_str(raw_texts_json)?;
    assert!(texts_value["texts"].is_array(), "Texts should be an array");
    assert_eq!(texts_value["mime_type"], "text/markdown", "MIME type should match");
    assert!(texts_value["chunk_capacity"].is_object(), "chunk_capacity should be an object");
    assert!(texts_value["chunk_overlap"].is_number(), "chunk_overlap should be a number");
    assert!(texts_value["chunk_batch_size"].is_number(), "chunk_batch_size should be a number");

    let images_value: serde_json::Value = serde_json::from_str(raw_images_json)?;
    assert!(images_value["images"].is_array(), "Images should be an array");
    assert_eq!(images_value["mime_type"], "image/jpeg", "MIME type should match");

    // Print example JSON structures for documentation
    println!("Example Globs JSON structure:\n{}", raw_globs_json);
    println!("\nExample Texts JSON structure:\n{}", raw_texts_json);
    println!("\nExample Images JSON structure:\n{}", raw_images_json);

    Ok(())
}
