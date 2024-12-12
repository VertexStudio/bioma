use bioma_actor::prelude::*;
use bioma_llm::pdf_analyzer::*;
use std::path::PathBuf;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Initialize the actor system
    let engine = Engine::test().await?;

    // Create actor ID for pdf analyzer
    let pdf_analyzer_id = ActorId::of::<PdfAnalyzer>("/pdf_analyzer");

    // Spawn and start the pdf analyzer actor
    let (mut pdf_analyzer_ctx, mut pdf_analyzer_actor) =
        Actor::spawn(engine.clone(), pdf_analyzer_id.clone(), PdfAnalyzer::default(), SpawnOptions::default()).await?;

    let pdf_analyzer_handle = tokio::spawn(async move {
        if let Err(e) = pdf_analyzer_actor.start(&mut pdf_analyzer_ctx).await {
            error!("PDF analyzer actor error: {}", e);
        }
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Spawn a relay actor
    let relay_id = ActorId::of::<Relay>("/relay");
    let (relay_ctx, _relay_actor) =
        Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await?;

    // Get workspace root
    let workspace_root = std::env::var("CARGO_MANIFEST_DIR")
        .map(std::path::PathBuf::from)
        .ok()
        .and_then(|path| path.parent().map(|p| p.to_path_buf()))
        .unwrap()
        .to_string_lossy()
        .to_string();

    // PDF file to analyze
    let file_path = PathBuf::from(format!("{}/assets/files/README.pdf", workspace_root));

    // Send the PDF for analysis
    let pdf_content = relay_ctx
        .send_and_wait_reply::<PdfAnalyzer, AnalyzePdf>(
            AnalyzePdf { file_path: file_path.clone() },
            &pdf_analyzer_id,
            SendOptions::default(),
        )
        .await?;

    info!("Analyzed PDF content:\n{}", pdf_content);

    pdf_analyzer_handle.abort();

    // Export the database for debugging
    dbg_export_db!(engine);

    Ok(())
}
