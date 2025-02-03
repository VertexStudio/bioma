use std::path::PathBuf;

use bioma_actor::prelude::*;
use bioma_tool::client::ModelContextProtocolClientActor;
use clap::Parser;
use cognition::tool::ToolClient;
use config::{Args as ConfigArgs, Config};
use ollama_rs::generation::tools::ToolInfo;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info};

pub mod config;
pub mod tool;
pub mod user;

#[derive(Parser)]
pub struct Args {
    /// tools_actor id
    pub tools_actor: String,

    /// Path to the config file
    pub config: Option<PathBuf>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CognitionClientActor {
    tools_clients: Vec<ToolClient>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ListTools;

impl Message<ListTools> for CognitionClientActor {
    type Response = Vec<ToolInfo>;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, msg: &ListTools) -> Result<(), Self::Error> {
        info!("{} Received message: {:?}", ctx.id(), msg);
        // ctx.reply(self.tools_clients.clone()).await?;
        Ok(())
    }
}

impl Actor for CognitionClientActor {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("{} Started", ctx.id());
        info!("{} Waiting for messages of type {}", ctx.id(), std::any::type_name::<ToolClient>());

        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(tool_client) = frame.is::<ListTools>() {
                let response = self.reply(ctx, &tool_client, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            }
        }
        info!("{} Finished", ctx.id());
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let args = Args::parse();

    let config = match args.config {
        Some(path) => {
            let config = ConfigArgs { config: Some(path) };

            config.load_config()?
        }
        None => Config::default(),
    };

    // Initialize engine
    let engine = Engine::connect(config.engine.clone()).await?;

    let mut clients = vec![];

    for tool in &config.tools {
        let hosting = tool.host;
        let server = tool.server.clone();
        let client_id = ActorId::of::<ModelContextProtocolClientActor>(format!("{}/{}", args.tools_actor, server.name));
        // If hosting, spawn client, which will spawn and host a ModelContextProtocol server
        let client_handle = if tool.host {
            debug!("Spawning ModelContextProtocolClient actor for client {}", client_id);
            let (mut client_ctx, mut client_actor) = Actor::spawn(
                engine.clone(),
                client_id.clone(),
                ModelContextProtocolClientActor::new(server.clone()),
                SpawnOptions::builder()
                    .exists(SpawnExistsOptions::Reset)
                    .health_config(
                        HealthConfig::builder().update_interval(std::time::Duration::from_secs(1).into()).build(),
                    )
                    .build(),
            )
            .await?;
            let client_id_spawn = client_id.clone();
            Some(tokio::spawn(async move {
                if let Err(e) = client_actor.start(&mut client_ctx).await {
                    error!("ModelContextProtocolClient actor error: {} for client {}", e, client_id_spawn);
                }
            }))
        } else {
            None
        };
        clients.push(ToolClient { hosting, server, client_id, _client_handle: client_handle, tools: vec![] });
    }
    // tools_hub.list_tools(&tools_user).await?; // TODO

    // Wait for interrupt signal
    tokio::signal::ctrl_c().await?;
    info!("Received shutdown signal, cleaning up...");

    Ok(())
}
