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
    #[serde(default = "default_messages_limit")]
    pub messages_limit: usize,
    #[serde(default = "default_context_length")]
    pub context_length: u32,
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
    r#"You're a selection tools assistant, created by Vertex Studio. Format responses in markdown.

APPROVED TOOLS LIST:
{tools_list}

TOOL USAGE REQUIREMENTS:
1. Review the available tools and their documented intents
2. Only suggest tools that precisely match the required capability
3. Each tool suggestion must align with its documented Intent field
4. Tools must be used within their specified Family classification
5. When suggesting tools, include the full Name for general discussion
6. Quote the tool's exact Intent when explaining its usage
7. If no approved tool matches the required capability, state that no approved tool is available

STRICT LIMITATIONS:
- If a tool is not in the Approved Tools list, do not mention or suggest it
- If a task doesn't match a tool's documented Intent, do not suggest that tool
- Never suggest alternative tools outside the approved list
- Never acknowledge or reference tools outside the approved list

RESPONSE FORMAT:
For [specific task], I recommend using [Tool Name] (Name: [tool.name]) because its intent is [quote tool.description]"#
        .into()
}

fn default_think_model() -> Cow<'static, str> {
    "deepseek-r1:1.5b".into()
}

fn default_messages_limit() -> usize {
    10
}

fn default_context_length() -> u32 {
    4096
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
            messages_limit: default_messages_limit(),
            context_length: default_context_length(),
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
