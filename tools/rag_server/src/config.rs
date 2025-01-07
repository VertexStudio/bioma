use anyhow::Result;
use bioma_tool::client::ServerConfig;
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::path::PathBuf;
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_engine_endpoint")]
    pub engine_endpoint: Cow<'static, str>,
    #[serde(default = "default_rag_endpoint")]
    pub rag_endpoint: Cow<'static, str>,
    #[serde(default = "default_chat_model")]
    pub chat_model: Cow<'static, str>,
    #[serde(default = "default_chat_prompt")]
    pub chat_prompt: Cow<'static, str>,
    pub tool_servers: Vec<ServerConfig>,
}

fn default_engine_endpoint() -> Cow<'static, str> {
    "ws://0.0.0.0:9123".into()
}

fn default_rag_endpoint() -> Cow<'static, str> {
    "http://0.0.0.0:5766".into()
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
            engine_endpoint: default_engine_endpoint(),
            rag_endpoint: default_rag_endpoint(),
            chat_model: default_chat_model(),
            chat_prompt: default_chat_prompt(),
            tool_servers: vec![],
        }
    }
}

#[derive(Parser)]
pub struct Args {
    #[arg(short, long)]
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
        info!("├─ Engine Endpoint: {}", config.engine_endpoint);
        info!("├─ Chat Model: {}", config.chat_model);
        info!("├─ Chat Prompt: {}...", config.chat_prompt.chars().take(50).collect::<String>());
        info!("├─ Tool Servers: {} configured", config.tool_servers.len());
        for (i, server) in config.tool_servers.iter().enumerate() {
            let prefix = if i == config.tool_servers.len() - 1 { "└──" } else { "├──" };
            info!("{}  {}", prefix, server.name);
        }
        Ok(config)
    }
}
