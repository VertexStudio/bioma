use bioma_actor::prelude::*;
use clap::Parser;
use client_config::Args;
use cognition::ToolsHub;
use tracing::info;

pub mod client_config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let args = Args::parse();
    let config = args.load_config()?;

    // Initialize engine
    let engine = Engine::connect(config.engine.clone()).await?;

    // Create ToolsHub instance
    let tools_hub = ToolsHub::new(&engine, config.tools.clone(), args.tools_hub_id.clone()).await?;

    // Spawn the ToolsHub actor
    let tools_hub_id_str = args.tools_hub_id.clone();
    let tools_hub_id = ActorId::of::<ToolsHub>(tools_hub_id_str);
    let (mut tools_hub_ctx, mut tools_hub_actor) = Actor::spawn(
        engine.clone(),
        tools_hub_id.clone(),
        tools_hub,
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await?;

    // Spawn the actor's main loop in a separate task
    let tools_hub_handle = tokio::spawn(async move {
        if let Err(e) = tools_hub_actor.start(&mut tools_hub_ctx).await {
            tracing::error!("ToolsHub actor error: {}", e);
        }
    });

    // Wait for interrupt signal
    tokio::signal::ctrl_c().await?;
    info!("Received shutdown signal, cleaning up...");

    // Abort the actor task
    tools_hub_handle.abort();

    Ok(())
}
