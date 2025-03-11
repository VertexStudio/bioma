use anyhow::Result;
use bioma_actor::EngineOptions;
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
    #[serde(default = "default_chat_endpoint")]
    pub chat_endpoint: Url,
    #[serde(default = "default_chat_model")]
    pub chat_model: Cow<'static, str>,
    #[serde(default = "default_chat_prompt")]
    pub chat_prompt: Cow<'static, str>,
    #[serde(default = "default_chat_messages_limit")]
    pub chat_messages_limit: usize,
    #[serde(default = "default_chat_context_length")]
    pub chat_context_length: u32,
    #[serde(default = "default_retrieve_limit")]
    pub retrieve_limit: usize,
}

fn default_engine() -> EngineOptions {
    EngineOptions::builder().endpoint("ws://0.0.0.0:9123".into()).build()
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

fn default_chat_messages_limit() -> usize {
    10
}

fn default_chat_context_length() -> u32 {
    4096
}

fn default_retrieve_limit() -> usize {
    5
}

impl Default for Config {
    fn default() -> Self {
        Self {
            engine: default_engine(),
            chat_endpoint: default_chat_endpoint(),
            chat_model: default_chat_model(),
            chat_prompt: default_chat_prompt(),
            chat_messages_limit: default_chat_messages_limit(),
            chat_context_length: default_chat_context_length(),
            retrieve_limit: default_retrieve_limit(),
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
        info!("├─ Chat Endpoint: {}", config.chat_endpoint);
        info!("├─ Chat Model: {}", config.chat_model);
        info!("├─ Chat Prompt: {}...", config.chat_prompt.chars().take(50).collect::<String>());
        info!("├─ Chat Messages Limit: {}", config.chat_messages_limit);
        info!("├─ Chat Context Length: {}", config.chat_context_length);
        info!("└─ Retrieve Limit: {}", config.retrieve_limit);

        Ok(config)
    }
}
