use anyhow::Result;
use bioma_actor::EngineOptions;
use bioma_mcp::client::ClientConfig as ToolClientConfig;
use clap::Parser;
use hostname;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::info;
use ulid::Ulid;
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    #[serde(default = "default_engine")]
    pub engine: EngineOptions,
    #[serde(default = "default_rag_endpoint")]
    pub rag_endpoint: Url,
    #[serde(flatten)]
    pub tools: ToolClientConfig,
}

fn default_engine() -> EngineOptions {
    EngineOptions::builder().endpoint("ws://0.0.0.0:9123".into()).build()
}

fn default_rag_endpoint() -> Url {
    Url::parse("http://0.0.0.0:5766").unwrap()
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self { engine: default_engine(), rag_endpoint: default_rag_endpoint(), tools: ToolClientConfig::default() }
    }
}

fn get_default_tools_hub_id() -> String {
    hostname::get()
        .map(|h| format!("/rag/tools_hub/{}", h.to_string_lossy()))
        .unwrap_or_else(|_| format!("/rag/tools_hub/{}", Ulid::new()))
}

#[derive(Parser)]
pub struct Args {
    pub config: Option<PathBuf>,
    #[clap(long, default_value_t = get_default_tools_hub_id())]
    pub tools_hub_id: String,
}

impl Args {
    pub fn load_config(&self) -> Result<ClientConfig> {
        let config: ClientConfig = match &self.config {
            Some(path) => {
                let config = std::fs::read_to_string(path)?;
                info!("Loaded config: {}", path.display());
                serde_json::from_str::<ClientConfig>(&config)?
            }
            None => {
                info!("Default config:");
                ClientConfig::default()
            }
        };
        info!("╔═══════════════════════════════════════════");
        info!("║ Tools Hub ID: {}", self.tools_hub_id);
        info!("╚═══════════════════════════════════════════");
        info!("├─ Engine Configuration:");
        info!("│  ├─ Endpoint: {}", config.engine.endpoint);
        info!("│  ├─ Namespace: {}", config.engine.namespace);
        info!("│  ├─ Database: {}", config.engine.database);
        info!("│  ├─ Username: {}", config.engine.username);
        info!("│  ├─ Output Directory: {}", config.engine.output_dir.display());
        info!("│  ├─ Local Store Directory: {}", config.engine.local_store_dir.display());
        info!("│  └─ HuggingFace Cache Directory: {}", config.engine.hf_cache_dir.display());
        info!("├─ RAG Endpoint: {}", config.rag_endpoint);
        info!("├─ Tool Servers: {} configured", config.tools.servers.len());
        for (i, tool) in config.tools.servers.iter().enumerate() {
            let prefix = if i == config.tools.servers.len() - 1 { "└──" } else { "├──" };
            info!("{}  {}", prefix, tool.name);
        }

        Ok(config)
    }
}
