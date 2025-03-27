use anyhow::Result;
use bioma_actor::{Actor, ActorId, Engine, Relay, SendOptions, SpawnOptions};
use bioma_llm::{
    chat::{Chat, ChatMessages},
    prelude::ChatMessage,
};
use bioma_mcp::{
    client::{Client, ClientError, ModelContextProtocolClient, ServerConfig, StdioConfig, TransportConfig},
    schema::{
        CallToolRequestParams, ClientCapabilities, ClientCapabilitiesRoots, CreateMessageRequestParams,
        CreateMessageResult, Implementation, ReadResourceRequestParams, Role, Root,
    },
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::io;
use std::io::Write;
use std::path::PathBuf;
use tracing::{error, info};

const DEFAULT_MODEL: &str = "llama3.2";

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, short)]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClientConfig {
    pub servers: Vec<ServerConfig>,
}

#[derive(Debug)]
struct SamplingChat {
    engine: Engine,
    chat_id: ActorId,
}

impl SamplingChat {
    pub async fn new(chat: Chat) -> Result<Self> {
        let engine = Engine::test().await?;

        let chat_id = ActorId::of::<Chat>("/llm/sampling");

        let (mut chat_ctx, mut chat_actor) =
            Actor::spawn(engine.clone(), chat_id.clone(), chat, SpawnOptions::default()).await?;

        let _chat_handle = tokio::spawn(async move {
            if let Err(e) = chat_actor.start(&mut chat_ctx).await {
                tracing::error!("Chat actor error: {}", e);
            }
        });

        Ok(Self { engine, chat_id })
    }
}

#[derive(Clone)]
pub struct ExampleMcpClient {
    server_configs: Vec<ServerConfig>,
    capabilities: ClientCapabilities,
    roots: Vec<Root>,
}

impl ModelContextProtocolClient for ExampleMcpClient {
    async fn get_server_configs(&self) -> Vec<ServerConfig> {
        self.server_configs.clone()
    }

    async fn get_capabilities(&self) -> ClientCapabilities {
        self.capabilities.clone()
    }

    async fn get_roots(&self) -> Vec<Root> {
        self.roots.clone()
    }

    async fn on_create_message(&self, params: CreateMessageRequestParams) -> Result<CreateMessageResult, ClientError> {
        info!("Params: {:#?}", params);

        print!("Accept request? (y/n): ");
        io::stdout().flush().expect("Failed to flush stdout");

        let mut input = String::new();
        io::stdin().read_line(&mut input).expect("Failed to read line");

        info!("User response: {}", input);

        if input.trim() != "y" {
            return Err(ClientError::SamplingRequestRejected);
        }

        info!("Starting sampling actor...");

        let model = match params.model_preferences {
            Some(model_preferences) => match model_preferences.hints {
                Some(hints) => hints.iter().find_map(|hint| hint.name.clone()).unwrap_or(DEFAULT_MODEL.to_string()),
                None => {
                    info!("Using default model");
                    DEFAULT_MODEL.to_string()
                }
            },
            None => {
                info!("Using default model");
                DEFAULT_MODEL.to_string()
            }
        };

        info!("Model: {}", model);

        let chat = Chat::builder().model(model.into()).build();

        let chat_sampling = SamplingChat::new(chat).await.map_err(|e| ClientError::Request(e.to_string().into()))?;

        let engine = chat_sampling.engine.clone();

        let relay_id = ActorId::of::<Relay>("/relay");
        let (relay_ctx, _relay_actor) = Actor::spawn(engine, relay_id.clone(), Relay, SpawnOptions::default())
            .await
            .map_err(|_| ClientError::Request("Error creating relay actor".into()))?;

        let chat_messages = params
            .messages
            .iter()
            .map(|sampling_message| ChatMessage {
                role: match sampling_message.role {
                    Role::User => ollama_rs::generation::chat::MessageRole::User,
                    Role::Assistant => ollama_rs::generation::chat::MessageRole::Assistant,
                },
                content: sampling_message.content.to_string(),
                tool_calls: vec![],
                images: None,
            })
            .collect();

        let chat_response = relay_ctx
            .send_and_wait_reply::<Chat, ChatMessages>(
                ChatMessages {
                    messages: chat_messages,
                    restart: true,
                    persist: false,
                    stream: false,
                    format: None,
                    tools: None,
                    options: None,
                },
                &chat_sampling.chat_id,
                SendOptions::builder().timeout(std::time::Duration::from_secs(600)).build(),
            )
            .await
            .map_err(|e| ClientError::Request(e.to_string().into()))?;

        let content = serde_json::to_value(chat_response.message.content).map_err(|e| ClientError::JsonError(e))?;

        Ok(CreateMessageResult {
            meta: None,
            content,
            model: chat_response.model,
            role: Role::Assistant,
            stop_reason: None,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    info!("Starting MCP client...");
    let args = Args::parse();

    let server_configs: Vec<ServerConfig> = if let Some(config_path) = &args.config {
        info!("Loading server configurations from: {}", config_path.display());
        let config_content =
            std::fs::read_to_string(config_path).map_err(|e| anyhow::anyhow!("Failed to read config file: {}", e))?;

        let client_config: ClientConfig =
            serde_json::from_str(&config_content).map_err(|e| anyhow::anyhow!("Failed to parse config file: {}", e))?;

        client_config.servers
    } else {
        info!("No configuration file provided. Using default stdio server configuration.");
        vec![ServerConfig::builder()
            .name("bioma-tool".to_string())
            .transport(TransportConfig::Stdio(StdioConfig {
                command: "target/release/examples/mcp_server".to_string(),
                args: vec!["stdio".to_string()],
            }))
            .request_timeout(60)
            .build()]
    };

    info!("Loaded {} server configurations", server_configs.len());
    for (i, server) in server_configs.iter().enumerate() {
        info!("Server {}: {}", i + 1, server.name);
    }

    let capabilities =
        ClientCapabilities { roots: Some(ClientCapabilitiesRoots { list_changed: Some(true) }), ..Default::default() };

    let client = ExampleMcpClient {
        server_configs,
        capabilities,
        roots: vec![Root { name: Some("workspace".to_string()), uri: "file:///workspace".to_string() }],
    };

    let mut client = Client::new(client).await?;

    info!("Initializing client...");

    let init_result = client
        .initialize(Implementation { name: "mcp_client_example".to_string(), version: "0.1.0".to_string() })
        .await?;
    info!("Server capabilities: {:?}", init_result);

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    client.initialized().await?;

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    info!("Listing prompts...");
    let prompts_result = client.list_prompts(None).await;
    match prompts_result {
        Ok(prompts_result) => info!("Available prompts: {:?}", prompts_result.prompts),
        Err(e) => error!("Error listing prompts: {:?}", e),
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    info!("Listing resources...");

    let resources_result = client.list_resources(None).await;

    match resources_result {
        Ok(resources_result) => {
            info!("Available resources: {:?}", resources_result.resources);

            if let Some(filesystem) = resources_result.resources.iter().find(|r| r.name == "filesystem") {
                info!("Found filesystem resource: {}", filesystem.uri);

                let readme_uri = "file:///bioma/README.md";
                info!("Reading file: {}", readme_uri);

                let readme_result =
                    client.read_resource(ReadResourceRequestParams { uri: readme_uri.to_string() }).await;
                match readme_result {
                    Ok(result) => {
                        if let Some(content) = result.contents.first() {
                            if let Some(text) = content.get("text").and_then(|t| t.as_str()) {
                                info!(
                                    "README.md content preview (first 100 chars): {}",
                                    text.chars().take(100).collect::<String>()
                                );
                            } else if let Some(blob) = content.get("blob").and_then(|b| b.as_str()) {
                                info!("README.md is a binary file with {} bytes", blob.len());
                            }
                        }
                    }
                    Err(e) => error!("Error reading README.md: {:?}", e),
                }

                let dir_uri = "file:///";
                info!("Reading directory: {}", dir_uri);

                let dir_result = client.read_resource(ReadResourceRequestParams { uri: dir_uri.to_string() }).await;
                match dir_result {
                    Ok(result) => {
                        info!("Directory contents:");
                        for content in result.contents {
                            if let Some(text) = content.get("text").and_then(|t| t.as_str()) {
                                info!("- {}", text);
                            }
                        }
                    }
                    Err(e) => error!("Error reading root directory: {:?}", e),
                }

                info!("Checking for resource templates...");
                let templates_result = client.list_resource_templates(None).await;
                match templates_result {
                    Ok(templates) => {
                        info!("Found {} resource templates", templates.resource_templates.len());
                        for template in templates.resource_templates {
                            info!("- Template: {} URI: {}", template.name, template.uri_template);
                        }

                        info!("Trying to subscribe to filesystem changes...");
                        let filesystem_uri = "file:///mcp_server.log";
                        match client.subscribe_resource(filesystem_uri.to_string()).await {
                            Ok(_) => info!("Successfully subscribed to filesystem changes at {}", filesystem_uri),
                            Err(e) => error!("Failed to subscribe to filesystem changes: {:?}", e),
                        }
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                        match client.unsubscribe_resource(filesystem_uri.to_string()).await {
                            Ok(_) => info!("Successfully unsubscribed from filesystem changes"),
                            Err(e) => error!("Failed to unsubscribe from filesystem changes: {:?}", e),
                        }
                    }
                    Err(e) => info!("Resource templates not supported: {:?}", e),
                }
            } else {
                info!("Filesystem resource not found, falling back to readme resource");

                if !resources_result.resources.is_empty() {
                    let read_result = client
                        .read_resource(ReadResourceRequestParams { uri: resources_result.resources[0].uri.clone() })
                        .await;

                    match read_result {
                        Ok(result) => info!("Resource content: {:?}", result),
                        Err(e) => error!("Error reading resource: {:?}", e),
                    }
                }
            }
        }
        Err(e) => error!("Error listing resources: {:?}", e),
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    info!("Listing tools...");

    let tools_result = client.list_tools(None).await;

    match tools_result {
        Ok(tools_result) => {
            info!("Available tools:");
            for tool in tools_result.tools {
                info!("- {}", tool.name);
            }
        }
        Err(e) => error!("Error listing tools: {:?}", e),
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    info!("Making sampling tool call...");
    let sampling_args = serde_json::json!({
        "messages": [{"content":"Explain the history of Rust programming language.", "role":"user"}],
        "max_tokens": 100,
        "models_suggestions": ["llama3.2"],
    });

    let sampling_call = CallToolRequestParams {
        name: "sampling".to_string(),
        arguments: serde_json::from_value(sampling_args).map_err(|e| ClientError::JsonError(e))?,
    };

    let sampling_result = client.call_tool(sampling_call).await?;

    info!("Sampling response: {:#?}", sampling_result);

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    info!("Making echo tool call...");

    let echo_args = serde_json::json!({
        "message": "Hello from MCP client!"
    });
    let echo_args = CallToolRequestParams {
        name: "echo".to_string(),
        arguments: serde_json::from_value(echo_args).map_err(|e| ClientError::JsonError(e))?,
    };
    let echo_result = client.call_tool(echo_args).await?;

    info!("Echo response: {:?}", echo_result);

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    info!("Updating roots...");
    let root = Root { name: Some("workspace".to_string()), uri: "file:///workspace".to_string() };
    client.add_root(root, None).await?;

    info!("Shutting down client...");
    client.close().await?;

    Ok(())
}
