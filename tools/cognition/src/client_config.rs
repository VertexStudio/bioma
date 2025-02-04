use anyhow::Result;
use bioma_actor::EngineOptions;
use bioma_tool::client::ClientConfig as ToolClientConfig;
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::info;
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    #[serde(default = "default_engine")]
    pub engine: EngineOptions,
    #[serde(default = "default_rag_endpoint")]
    pub rag_endpoint: Url,
    pub tools: Vec<ToolClientConfig>,
}

fn default_engine() -> EngineOptions {
    EngineOptions::builder().endpoint("ws://0.0.0.0:9123".into()).build()
}

fn default_rag_endpoint() -> Url {
    Url::parse("http://0.0.0.0:5766").unwrap()
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            engine: default_engine(),
            rag_endpoint: default_rag_endpoint(),
            tools: vec![],
        }
    }
}

#[derive(Parser)]
pub struct Args {
    pub config: Option<PathBuf>,
    #[clap(long, default_value = "/rag/tools_hub")]
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
        info!("├─ Engine Configuration:");
        info!("│  ├─ Endpoint: {}", config.engine.endpoint);
        info!("│  ├─ Namespace: {}", config.engine.namespace);
        info!("│  ├─ Database: {}", config.engine.database);
        info!("│  ├─ Username: {}", config.engine.username);
        info!("│  ├─ Output Directory: {}", config.engine.output_dir.display());
        info!("│  ├─ Local Store Directory: {}", config.engine.local_store_dir.display());
        info!("│  └─ HuggingFace Cache Directory: {}", config.engine.hf_cache_dir.display());
        info!("├─ RAG Endpoint: {}", config.rag_endpoint);
        info!("├─ Tool Servers: {} configured", config.tools.len());
        for (i, tool) in config.tools.iter().enumerate() {
            let prefix = if i == config.tools.len() - 1 { "└──" } else { "├──" };
            info!("{}  {}", prefix, tool.server.name);
        }

        Ok(config)
    }
}
