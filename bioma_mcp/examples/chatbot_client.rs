use std::io::{self, Write};

use anyhow::Result;
use bioma_mcp::{
    client::{Client, ClientError, ModelContextProtocolClient, ServerConfig, StdioConfig, TransportConfig},
    schema::{
        CallToolRequestParams, ClientCapabilities, CreateMessageRequestParams, CreateMessageResult, Implementation,
        Role, SamplingMessage,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{error, info};

const DEFAULT_MODEL: &str = "llama3.2";

#[derive(Clone)]
struct ChatBotClient {
    server_configs: Vec<ServerConfig>,
    capabilities: ClientCapabilities,
}

#[derive(Serialize, Deserialize, Debug)]
struct OllamaRequest {
    model: String,
    messages: Vec<SamplingMessage>,
    stream: bool,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct OllamaResponse {
    model: String,
    created_at: String,
    message: OllamaMessage,
    done_reason: String,
    done: bool,
    total_duration: u64,
    load_duration: u64,
    prompt_eval_count: u32,
    prompt_eval_duration: u64,
    eval_count: u32,
    eval_duration: u64,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

impl ChatBotClient {
    fn new() -> Self {
        let server_configs = vec![ServerConfig::builder()
            .name("chatbot-server".to_string())
            .transport(TransportConfig::Stdio(StdioConfig {
                command: "target/release/examples/chatbot_server".to_string(),
                args: vec!["studio".to_string()],
            }))
            .request_timeout(120)
            .build()];

        Self { server_configs, capabilities: ClientCapabilities::default() }
    }
}

impl ModelContextProtocolClient for ChatBotClient {
    async fn get_server_configs(&self) -> Vec<ServerConfig> {
        self.server_configs.clone()
    }
    async fn get_capabilities(&self) -> ClientCapabilities {
        self.capabilities.clone()
    }
    async fn get_roots(&self) -> Vec<bioma_mcp::schema::Root> {
        vec![]
    }
    async fn on_create_message(&self, params: CreateMessageRequestParams) -> Result<CreateMessageResult, ClientError> {
        info!("Received create message request with {} messages", params.messages.len());
        info!("Params: {:#?}", params);

        let model = match params.model_preferences {
            Some(model_prefs) => match &model_prefs.hints {
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

        // Prepare the request body
        let body = OllamaRequest { model: model.clone(), messages: params.messages, stream: false };

        // Create HTTP client and send request
        let client = reqwest::Client::new();
        let res = client.post("http://localhost:11434/api/chat").json(&body).send().await;

        // Handle the response
        let llm_response = match res {
            Ok(res) => res.text().await.unwrap_or_else(|_| "Error reading response".to_string()),
            Err(err) => format!("Error while sending request: {}", err),
        };

        Ok(CreateMessageResult {
            meta: None,
            content: serde_json::to_value(llm_response).unwrap(),
            model: body.model,
            role: Role::Assistant,
            stop_reason: None,
        })
    }
}

fn read_user_input(prompt: &str) -> io::Result<String> {
    info!("{}", prompt);
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    info!("Starting ChatBot client...");

    // Create client
    let mut client = Client::new(ChatBotClient::new()).await?;

    // Init client
    let init_result =
        client.initialize(Implementation { name: "chatbot-client".to_string(), version: "0.1.0".to_string() }).await?;

    info!("Server capabilities: {:?}", init_result);

    // let messages = vec!["hola", "como estas", "me gusta programar en rust", "adios"];

    let mut conversation_history: Vec<(String, String)> = Vec::new();

    info!("\nWelcome to the ChatBot! (Write exit or ctrl+c to close the program)\n");

    loop {
        // Read the user input
        info!("\n=== User Message ===");
        let user_message = match read_user_input("Usuario >") {
            Ok(msg) if msg.trim().is_empty() => continue,
            Ok(msg) if msg.to_lowercase() == "exit" => break,
            Ok(msg) => msg,
            Err(e) => {
                error!("Error reading the user input: {}", e);
                continue;
            }
        };

        let args = CallToolRequestParams {
            name: "chat".to_string(),
            arguments: serde_json::from_value(json!({
                "message": user_message
            }))
            .map_err(|e| ClientError::JsonError(e))?,
        };

        match client.call_tool(args).await {
            Ok(result) => {
                if let Some(content) = result.content.first() {
                    if let Some(text) = content.get("text") {
                        // let bot_response = text.as_str().unwrap_or("").to_string();
                        if let Ok(inner_json_str) = serde_json::from_str::<String>(text.as_str().unwrap_or("")) {
                            if let Ok(llm_response) = serde_json::from_str::<OllamaResponse>(&inner_json_str) {
                                let bot_response = llm_response.message.content;
                                info!("Bot > {}", bot_response);

                                conversation_history.push((user_message.clone(), bot_response));

                                if let Ok(command) =
                                    read_user_input("Press 'h' to see the history or Enter to continue")
                                {
                                    if command.to_lowercase() == "h" {
                                        info!("\n===Conversation history===");
                                        for (i, (user, bot)) in conversation_history.iter().enumerate() {
                                            info!("\nInteraction {}:", i + 1);
                                            info!("User > {}", user);
                                            info!("Bot > {}", bot);
                                        }
                                        info!("\n=========================\n");
                                    }
                                }
                            }
                        }
                    } else {
                        info!("Response content did not contain text field: {:?}", content);
                    }
                }
            }
            Err(err) => error!("Error calling chatbot: {}", err),
        }
    }

    info!("Thanks for use our chat! Here is the conversation resume:");
    for (i, (user, bot)) in conversation_history.iter().enumerate() {
        info!("\nInteraction {}:", i + 1);
        info!("User > {}", user);
        info!("Bot > {}", bot);
    }
    info!("Chat session completed");
    Ok(())
}
