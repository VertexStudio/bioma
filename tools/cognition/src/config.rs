use anyhow::Result;
use bioma_actor::EngineOptions;
use bioma_tool::client::ClientConfig;
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::path::PathBuf;
use tracing::info;
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_engine")]
    pub engine: EngineOptions,
    #[serde(default = "default_rag_endpoint")]
    pub rag_endpoint: Url,
    #[serde(default = "default_chat_endpoint")]
    pub chat_endpoint: Url,
    #[serde(default = "default_chat_model")]
    pub chat_model: Cow<'static, str>,
    #[serde(default = "default_chat_prompt")]
    pub chat_prompt: Cow<'static, str>,
    pub tools: Vec<ClientConfig>,
}

fn default_engine() -> EngineOptions {
    EngineOptions::builder().endpoint("ws://0.0.0.0:9123".into()).build()
}

fn default_rag_endpoint() -> Url {
    Url::parse("http://0.0.0.0:5766").unwrap()
}

fn default_chat_endpoint() -> Url {
    Url::parse("http://0.0.0.0:11434").unwrap()
}

fn default_chat_model() -> Cow<'static, str> {
    "llama3.2-vision".into()
}

fn default_chat_prompt() -> Cow<'static, str> {
    r#"You are, Bioma, a helpful assistant. Your creator is Vertex Studio, a games and 
simulation company. Format your response in markdown. Use the following context to 
answer the user's query:

"#
    .into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            engine: default_engine(),
            rag_endpoint: default_rag_endpoint(),
            chat_endpoint: default_chat_endpoint(),
            chat_model: default_chat_model(),
            chat_prompt: default_chat_prompt(),
            tools: vec![],
        }
    }
}

#[derive(Parser)]
pub struct Args {
    pub config: Option<PathBuf>,
}

impl Args {
    pub fn load_config(&self) -> Result<Config> {
        let config: Config = match &self.config {
            Some(path) => {
                let config = std::fs::read_to_string(path)?;
                info!("Loaded config: {}", path.display());
                serde_json::from_str::<Config>(&config)?
            }
            None => {
                info!("Default config:");
                Config::default()
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
        info!("├─ Chat Endpoint: {}", config.chat_endpoint);
        info!("├─ Chat Model: {}", config.chat_model);
        info!("├─ Chat Prompt: {}...", config.chat_prompt.chars().take(50).collect::<String>());
        info!("├─ Tool Servers: {} configured", config.tools.len());
        for (i, tool) in config.tools.iter().enumerate() {
            let prefix = if i == config.tools.len() - 1 { "└──" } else { "├──" };
            info!("{}  {}", prefix, tool.server.name);
        }

        Ok(config)
    }
}
