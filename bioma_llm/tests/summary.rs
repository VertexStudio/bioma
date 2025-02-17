use base64::Engine as _;
use bioma_actor::prelude::*;
use bioma_llm::{prelude::*, summary::SummarizeContent};
use std::fs;
use tempfile;
use test_log::test;
use tracing::error;

#[derive(thiserror::Error, Debug)]
enum TestError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Summary error: {0}")]
    Summary(#[from] SummaryError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[test(tokio::test)]
async fn test_text_summarization() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Create a test text
    let text = r#"# Important Document
    
This is a detailed document about a complex topic.
It contains multiple paragraphs of information that should be summarized.

## First Section
The first section discusses key concepts and ideas.
These concepts are fundamental to understanding the topic.

## Second Section
The second section builds upon the first section.
It provides practical examples and use cases.
"#;

    // Initialize the Summary actor
    let summary_id = ActorId::of::<Summary>("/summary");
    let (mut summary_ctx, mut summary_actor) = Actor::spawn(
        engine.clone(),
        summary_id.clone(),
        Summary::builder().chat(Chat::builder().model(std::borrow::Cow::Borrowed("llama2")).build()).build(),
        SpawnOptions::default(),
    )
    .await?;

    let summary_handle = tokio::spawn(async move {
        if let Err(e) = summary_actor.start(&mut summary_ctx).await {
            error!("Summary actor error: {}", e);
        }
    });

    // Give actor time to initialize
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Request text summarization
    let response = relay_ctx
        .send_and_wait_reply::<Summary, SummarizeText>(
            SummarizeText { content: SummarizeContent::Text(text.to_string()), uri: "test_document.md".to_string() },
            &summary_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(30)).build(),
        )
        .await?;

    // Verify summary format and content
    assert!(response.summary.contains("**URI**:"), "Summary should contain URI");
    assert!(response.summary.contains("**Summary**:"), "Summary should contain summary section");
    assert!(response.summary.contains("test_document.md"), "Summary should contain the correct URI");

    // Cleanup
    summary_handle.abort();
    Ok(())
}

#[test(tokio::test)]
async fn test_image_summarization() -> Result<(), TestError> {
    let engine = Engine::test().await?;
    let temp_dir = tempfile::tempdir()?;

    // Copy test image to temp directory
    let test_image = "../assets/images/elephant.jpg";
    let dest_path = temp_dir.path().join("elephant.jpg");
    fs::copy(test_image, &dest_path)?;

    // Read and encode image
    let image_data = fs::read(&dest_path)?;
    let base64_data = base64::engine::general_purpose::STANDARD.encode(image_data);

    // Initialize the Summary actor
    let summary_id = ActorId::of::<Summary>("/summary");
    let (mut summary_ctx, mut summary_actor) = Actor::spawn(
        engine.clone(),
        summary_id.clone(),
        Summary::builder().chat(Chat::builder().model(std::borrow::Cow::Borrowed("llama2")).build()).build(),
        SpawnOptions::default(),
    )
    .await?;

    let summary_handle = tokio::spawn(async move {
        if let Err(e) = summary_actor.start(&mut summary_ctx).await {
            error!("Summary actor error: {}", e);
        }
    });

    // Give actor time to initialize
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Request image summarization
    let response = relay_ctx
        .send_and_wait_reply::<Summary, SummarizeText>(
            SummarizeText { content: SummarizeContent::Image(base64_data), uri: "elephant.jpg".to_string() },
            &summary_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(30)).build(),
        )
        .await?;

    // Verify summary format and content
    assert!(response.summary.contains("**URI**:"), "Summary should contain URI");
    assert!(response.summary.contains("**Summary**:"), "Summary should contain summary section");
    assert!(response.summary.contains("elephant.jpg"), "Summary should contain the correct URI");

    // Cleanup
    summary_handle.abort();
    temp_dir.close()?;
    Ok(())
}

#[test(tokio::test)]
async fn test_summary_error_handling() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Initialize the Summary actor
    let summary_id = ActorId::of::<Summary>("/summary");
    let (mut summary_ctx, mut summary_actor) = Actor::spawn(
        engine.clone(),
        summary_id.clone(),
        Summary::builder().chat(Chat::builder().model(std::borrow::Cow::Borrowed("llama2")).build()).build(),
        SpawnOptions::default(),
    )
    .await?;

    let summary_handle = tokio::spawn(async move {
        if let Err(e) = summary_actor.start(&mut summary_ctx).await {
            error!("Summary actor error: {}", e);
        }
    });

    // Give actor time to initialize
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Test with empty text
    let response = relay_ctx
        .send_and_wait_reply::<Summary, SummarizeText>(
            SummarizeText { content: SummarizeContent::Text("".to_string()), uri: "empty.txt".to_string() },
            &summary_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(30)).build(),
        )
        .await?;

    // Even empty text should produce a valid summary format
    assert!(response.summary.contains("**URI**:"), "Summary should contain URI");
    assert!(response.summary.contains("**Summary**:"), "Summary should contain summary section");

    // Test with invalid base64 for image
    let response = relay_ctx
        .send_and_wait_reply::<Summary, SummarizeText>(
            SummarizeText {
                content: SummarizeContent::Image("invalid_base64".to_string()),
                uri: "invalid.jpg".to_string(),
            },
            &summary_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(30)).build(),
        )
        .await?;

    // Invalid image should still produce a valid summary format with error indication
    assert!(response.summary.contains("**URI**:"), "Summary should contain URI");
    assert!(response.summary.contains("**Summary**:"), "Summary should contain summary section");

    // Cleanup
    summary_handle.abort();
    Ok(())
}

#[test(tokio::test)]
async fn test_long_text_truncation() -> Result<(), TestError> {
    let engine = Engine::test().await?;

    // Create a very long text that exceeds MAX_TEXT_LENGTH
    let long_text = "This is a test paragraph.\n".repeat(1000);

    // Initialize the Summary actor
    let summary_id = ActorId::of::<Summary>("/summary");
    let (mut summary_ctx, mut summary_actor) = Actor::spawn(
        engine.clone(),
        summary_id.clone(),
        Summary::builder().chat(Chat::builder().model(std::borrow::Cow::Borrowed("llama2")).build()).build(),
        SpawnOptions::default(),
    )
    .await?;

    let summary_handle = tokio::spawn(async move {
        if let Err(e) = summary_actor.start(&mut summary_ctx).await {
            error!("Summary actor error: {}", e);
        }
    });

    // Give actor time to initialize
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Request summarization of long text
    let response = relay_ctx
        .send_and_wait_reply::<Summary, SummarizeText>(
            SummarizeText { content: SummarizeContent::Text(long_text), uri: "long_text.txt".to_string() },
            &summary_id,
            SendOptions::builder().timeout(std::time::Duration::from_secs(30)).build(),
        )
        .await?;

    // Verify summary format and truncation indication
    assert!(response.summary.contains("**URI**:"), "Summary should contain URI");
    assert!(response.summary.contains("**Summary**:"), "Summary should contain summary section");
    assert!(response.summary.contains("text truncated"), "Summary should indicate text was truncated");

    // Cleanup
    summary_handle.abort();
    Ok(())
}
