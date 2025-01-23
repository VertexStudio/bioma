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
    #[serde(default = "default_think_prompt")]
    pub think_prompt: Cow<'static, str>,
    #[serde(default = "default_think_model")]
    pub think_model: Cow<'static, str>,
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

fn default_think_prompt() -> Cow<'static, str> {
    r#"You are a tool selection and planning assistant. Analyze the query.

First, determine if the query requires the use of external tools to accomplish.

- If the query requires tools, create a structured execution plan following this format:

  1. Task name
  - Tools: [exact tool names to use]
  - Action: [what these tools will accomplish]

  Organize tasks in sequential order.
  Each task should clearly state which tools are needed and why.
  If a task requires multiple tools, explain their combined usage.

- If the query does not require tools, respond with: "No tools needed for this query." and do not create a task plan.

"#
    .into()
}

fn default_think_model() -> Cow<'static, str> {
    "deepseek-r1:1.5b".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            engine: default_engine(),
            rag_endpoint: default_rag_endpoint(),
            chat_endpoint: default_chat_endpoint(),
            chat_model: default_chat_model(),
            chat_prompt: default_chat_prompt(),
            think_prompt: default_think_prompt(),
            think_model: default_think_model(),
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
        info!("├─ Think Prompt: {}...", config.think_prompt.chars().take(50).collect::<String>());
        info!("├─ Think Model: {}", config.think_model);
        info!("├─ Tool Servers: {} configured", config.tools.len());
        for (i, tool) in config.tools.iter().enumerate() {
            let prefix = if i == config.tools.len() - 1 { "└──" } else { "├──" };
            info!("{}  {}", prefix, tool.server.name);
        }

        Ok(config)
    }
}
