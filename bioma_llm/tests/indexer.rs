use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use tempfile;
use test_log::test;
use tracing::error;

#[derive(thiserror::Error, Debug)]
enum TestError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Indexer error: {0}")]
    Indexer(#[from] IndexerError),
}

#[test(tokio::test)]
async fn test_delete_source_with_files() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Create temporary test files
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path1 = temp_dir.path().join("test1.txt");
    let file_path2 = temp_dir.path().join("test2.txt");

    tokio::fs::write(&file_path1, "Test content 1").await.unwrap();
    tokio::fs::write(&file_path2, "Test content 2").await.unwrap();

    // Spawn the indexer actor
    let indexer_id = ActorId::of::<Indexer>("/indexer");
    let (mut indexer_ctx, mut indexer_actor) =
        Actor::spawn(engine.clone(), indexer_id.clone(), Indexer::default(), SpawnOptions::default()).await?;

    let indexer_handle = tokio::spawn(async move {
        if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
            error!("Indexer actor error: {}", e);
        }
    });

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Index the files
    let globs = vec![temp_dir.path().join("*.txt").to_string_lossy().into_owned()];
    let index_result = relay_ctx
        .send_and_wait_reply::<Indexer, IndexGlobs>(
            IndexGlobs::builder().globs(globs).build(),
            &indexer_id,
            SendOptions::default(),
        )
        .await?;

    println!("index_result: {:?}", index_result);

    assert_eq!(index_result.indexed, 2, "Expected 2 files to be indexed");

    // Delete one of the files and its source
    let delete_result = relay_ctx
        .send_and_wait_reply::<Indexer, DeleteSource>(
            DeleteSource { source: file_path1.to_string_lossy().into_owned() },
            &indexer_id,
            SendOptions::default(),
        )
        .await?;

    // Verify deletion results
    assert!(delete_result.deleted_embeddings > 0, "Expected at least one embedding to be deleted");
    assert!(
        delete_result.deleted_sources.iter().any(|s| s.uri == file_path1.to_string_lossy()),
        "Expected file1 to be in deleted sources"
    );

    // Verify the file was actually deleted
    assert!(!file_path1.exists(), "Expected file1 to be deleted from filesystem");
    assert!(file_path2.exists(), "Expected file2 to still exist in filesystem");

    // Cleanup
    indexer_handle.abort();
    temp_dir.close().unwrap();

    Ok(())
}
