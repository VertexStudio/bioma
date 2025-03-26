use anyhow::Result;
use bioma_actor::{Actor, ActorId, Engine, Relay, SendOptions, SpawnOptions};
use bioma_llm::{
    chat::{Chat, ChatMessages},
    prelude::ChatMessage,
};
use bioma_mcp::{
    client::{Client, ModelContextProtocolClient, ServerConfig, SseConfig, StdioConfig, TransportConfig, WsConfig},
    schema::{
        CallToolRequestParams, ClientCapabilities, ClientCapabilitiesRoots, CreateMessageRequestParams,
        CreateMessageResult, Implementation, ReadResourceRequestParams, Role, Root, SamplingMessage,
    },
};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::io;
use std::{collections::HashMap, io::Write};
use tracing::{error, info};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    transport: Transport,
}

#[derive(Subcommand)]
enum Transport {
    Stdio {
        command: String,

        #[arg(num_args = 0.., value_delimiter = ' ')]
        args: Option<Vec<String>>,
    },

    Sse {
        #[arg(long, short, default_value = "http://127.0.0.1:8090")]
        endpoint: String,
    },

    Ws {
        #[arg(long, short, default_value = "ws://127.0.0.1:9090")]
        endpoint: String,
    },
}

#[derive(Debug)]
struct SamplingChat {
    engine: Engine,
    chat_handle: tokio::task::JoinHandle<()>,
}

impl SamplingChat {
    /// Creates a new ChatSampling instance
    pub async fn new() -> Self {
        let engine = Engine::test().await.unwrap();

        // Create a single Chat actor for all requests
        let chat_id = ActorId::of::<Chat>("/llm/sampling");
        let chat = Chat::default();

        // Spawn the Chat actor
        let (mut chat_ctx, mut chat_actor) =
            Actor::spawn(engine.clone(), chat_id.clone(), chat, SpawnOptions::default()).await.unwrap();

        // Start the Chat actor in a separate task
        let chat_handle = tokio::spawn(async move {
            if let Err(e) = chat_actor.start(&mut chat_ctx).await {
                tracing::error!("Chat actor error: {}", e);
            }
        });

        Self { engine, chat_handle }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct OllamaRequest {
    model: String,
    messages: Vec<SamplingMessage>,
    stream: bool,
}

#[derive(Clone)]
pub struct ExampleMcpClient {
    server_config: ServerConfig,
    capabilities: ClientCapabilities,
    roots: Vec<Root>,
    engine: Engine,
}

impl ModelContextProtocolClient for ExampleMcpClient {
    async fn get_server_config(&self) -> ServerConfig {
        self.server_config.clone()
    }

    async fn get_capabilities(&self) -> ClientCapabilities {
        self.capabilities.clone()
    }

    async fn get_roots(&self) -> Vec<Root> {
        self.roots.clone()
    }

    async fn on_create_message(&self, params: CreateMessageRequestParams) -> CreateMessageResult {
        info!("Params: {:#?}", params);

        print!("Accept request? (y/n): ");
        io::stdout().flush().expect("Failed to flush stdout");

        let mut input = String::new();
        io::stdin().read_line(&mut input).expect("Failed to read line");

        info!("User response: {}", input);

        let engine = self.engine.clone();

        let relay_id = ActorId::of::<Relay>("/relay");
        let (relay_ctx, _relay_actor) =
            Actor::spawn(engine.clone(), relay_id.clone(), Relay, SpawnOptions::default()).await.unwrap();

        let chat_id = ActorId::of::<Relay>("/llm/sampling");

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
                &chat_id,
                SendOptions::builder().timeout(std::time::Duration::from_secs(600)).build(),
            )
            .await
            .unwrap();

        CreateMessageResult {
            meta: None,
            content: serde_json::to_value(chat_response.message.content).unwrap(),
            model: "llama3.2".to_string(),
            role: Role::Assistant,
            stop_reason: None,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    info!("Starting MCP client...");
    let args = Args::parse();

    let server = match &args.transport {
        Transport::Stdio { command, args } => {
            info!("Starting to MCP server process with command: {}", command);
            ServerConfig::builder()
                .name("bioma-tool".to_string())
                .transport(TransportConfig::Stdio(StdioConfig {
                    command: command.clone(),
                    args: args.clone().unwrap_or_default(),
                }))
                .request_timeout(60)
                .build()
        }
        Transport::Sse { endpoint } => {
            info!("Connecting to MCP server at {}", endpoint);
            ServerConfig::builder()
                .name("bioma-tool".to_string())
                .transport(TransportConfig::Sse(SseConfig::builder().endpoint(endpoint.clone()).build()))
                .build()
        }
        Transport::Ws { endpoint } => {
            info!("Connecting to MCP server at {}", endpoint);
            ServerConfig::builder()
                .name("bioma-tool".to_string())
                .transport(TransportConfig::Ws(WsConfig { endpoint: endpoint.clone() }))
                .build()
        }
    };

    let capabilities =
        ClientCapabilities { roots: Some(ClientCapabilitiesRoots { list_changed: Some(true) }), ..Default::default() };

    info!("Starting sampling actor...");
    let chat_sampling = SamplingChat::new().await;

    let client = ExampleMcpClient {
        server_config: server,
        capabilities,
        roots: vec![Root { name: Some("workspace".to_string()), uri: "file:///workspace".to_string() }],
        engine: chat_sampling.engine.clone(),
    };

    let mut client = Client::new(client).await?;

    info!("Initializing client...");

    let init_result = client
        .initialize(Implementation { name: "mcp_client_example".to_string(), version: "0.1.0".to_string() })
        .await?;

    info!("Server capabilities: {:?}", init_result.capabilities);

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

    info!("Making sampling tool call...");
    let sampling_args = serde_json::json!({
        "query": "Why the sky is blue?"
    });

    let sampling_call = CallToolRequestParams {
        name: "sampling".to_string(),
        arguments: serde_json::from_value(sampling_args).unwrap(),
    };

    let sampling_result = client.call_tool(sampling_call).await?;

    info!("Sampling response: {:#?}", sampling_result);

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    info!("Making echo tool call...");

    let echo_args = serde_json::json!({
        "message": "Hello from MCP client!"
    });
    let echo_args =
        CallToolRequestParams { name: "echo".to_string(), arguments: serde_json::from_value(echo_args).unwrap() };
    let echo_result = client.call_tool(echo_args).await?;

    info!("Echo response: {:?}", echo_result);

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    info!("Updating roots...");
    let roots = HashMap::from([(
        "workspace".to_string(),
        Root { name: Some("workspace".to_string()), uri: "file:///workspace".to_string() },
    )]);

    client.update_roots(roots).await?;

    info!("Shutting down client...");
    client.close().await?;
    chat_sampling.chat_handle.abort();

    Ok(())
}
