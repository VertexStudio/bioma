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
            DeleteSource { sources: vec![glob_path] },
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
