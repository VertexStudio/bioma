use bioma_actor::prelude::*;
use bioma_llm::{prelude::*, retriever::ListSources};
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
}

#[test(tokio::test)]
async fn test_indexer_basic_text() -> Result<(), TestError> {
    let engine = Engine::test().await?;
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
        .send_and_wait_reply::<Indexer, IndexGlobs>(
            IndexGlobs::builder().globs(globs.clone()).build(),
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
        .send_and_wait_reply::<Indexer, IndexGlobs>(
            IndexGlobs::builder().globs(globs).build(),
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
    let engine = Engine::test().await?;
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
    let chunk_capacity = 100..200; // Small chunks for testing
    let chunk_overlap = 50;
    let chunk_batch_size = 10;

    let index_result = relay_ctx
        .send_and_wait_reply::<Indexer, IndexGlobs>(
            IndexGlobs::builder()
                .globs(globs)
                .chunk_capacity(chunk_capacity)
                .chunk_overlap(chunk_overlap)
                .chunk_batch_size(chunk_batch_size)
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
    let engine = Engine::test().await?;
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
        .send_and_wait_reply::<Indexer, IndexGlobs>(
            IndexGlobs::builder().globs(vec![glob_path.clone()]).build(),
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
            DeleteSource { sources: vec!["/global".to_string()] },
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
async fn test_indexer_with_summary() -> Result<(), TestError> {
    let engine = Engine::test().await?;
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

    // Index with summary generation enabled
    let globs = vec![temp_dir.path().join("*.md").to_string_lossy().into_owned()];
    let index_result = relay_ctx
        .send_and_wait_reply::<Indexer, IndexGlobs>(
            IndexGlobs::builder().globs(globs).summarize(true).build(),
            &indexer_id,
            SendOptions::default(),
        )
        .await?;

    assert_eq!(index_result.indexed, 1, "Expected 1 file to be indexed");

    // Verify summary was stored in the database
    let db = engine.db();
    let query = "SELECT summary FROM source WHERE uri = $uri";
    let mut results =
        db.lock().await.query(query).bind(("uri", "document.md")).await.map_err(SystemActorError::from)?;

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
    let engine = Engine::test().await?;
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

    // Index with summary generation disabled
    let globs = vec![temp_dir.path().join("*.md").to_string_lossy().into_owned()];
    let index_result = relay_ctx
        .send_and_wait_reply::<Indexer, IndexGlobs>(
            IndexGlobs::builder().globs(globs).summarize(false).build(),
            &indexer_id,
            SendOptions::default(),
        )
        .await?;

    assert_eq!(index_result.indexed, 1, "Expected 1 file to be indexed");

    // Verify no summary was stored in the database
    let db = engine.db();
    let query = "SELECT summary FROM source WHERE uri = $uri";
    let mut results =
        db.lock().await.query(query).bind(("uri", "no_summary.md")).await.map_err(SystemActorError::from)?;

    let summary: Option<String> =
        results.take::<Vec<Option<String>>>(0).map_err(SystemActorError::from)?.pop().flatten();
    assert!(summary.is_none(), "Summary should not be stored when summarize is false");

    // Cleanup
    indexer_handle.abort();
    temp_dir.close()?;

    Ok(())
}

#[test(tokio::test)]
async fn test_indexer_with_images() -> Result<(), TestError> {
    let engine = Engine::test().await?;
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
        Summary::builder().chat(Chat::builder().model(std::borrow::Cow::Borrowed("llama2")).build()).build();

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

    // Index with summary generation enabled
    let globs = vec![
        temp_dir.path().join("*.jpg").to_string_lossy().into_owned(),
        temp_dir.path().join("*.png").to_string_lossy().into_owned(),
    ];

    let index_result = relay_ctx
        .send_and_wait_reply::<Indexer, IndexGlobs>(
            IndexGlobs::builder().globs(globs).summarize(true).build(),
            &indexer_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(30)).build(),
        )
        .await?;

    assert_eq!(index_result.indexed, 2, "Expected 2 images to be indexed");
    assert_eq!(index_result.cached, 0, "Expected no cached images");

    // Verify summaries were stored in the database
    let db = engine.db();
    let query = "SELECT uri, summary FROM source WHERE uri LIKE $pattern";
    let mut results = db.lock().await.query(query).bind(("pattern", "%.jpg")).await.map_err(SystemActorError::from)?;

    let summaries: Vec<(String, Option<String>)> =
        results.take::<Vec<(String, Option<String>)>>(0).map_err(SystemActorError::from)?;
    assert!(!summaries.is_empty(), "Should find at least one image summary");

    for (uri, summary) in summaries {
        assert!(summary.is_some(), "Summary should be stored for {}", uri);
        let summary = summary.unwrap();
        assert!(summary.contains("**URI**:"), "Summary should contain URI");
        assert!(summary.contains("**Summary**:"), "Summary should contain summary section");
        println!("Summary content for {}: {}", uri, summary);
    }

    // Cleanup
    indexer_handle.abort();
    temp_dir.close()?;

    Ok(())
}
