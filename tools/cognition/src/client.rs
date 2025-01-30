use bioma_actor::prelude::*;
use clap::Parser;
use config::Args;
use tool::ToolsHub;
use tracing::info;
use user::UserActor;

pub mod config;
pub mod tool;
pub mod user;

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

    // Tools setup
    let tools_user = UserActor::new(&engine, args.tools_actor).await?;
    let mut tools_hub = ToolsHub::new();
    for tool in &config.tools {
        tools_hub.add_tool(&engine, tool.clone(), "/rag".into()).await?;
    }
    tools_hub.list_tools(&tools_user).await?;

    // Wait for interrupt signal
    tokio::signal::ctrl_c().await?;
    info!("Received shutdown signal, cleaning up...");

    Ok(())
}
