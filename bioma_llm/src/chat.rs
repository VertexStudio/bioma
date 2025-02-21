use bioma_actor::prelude::*;
use ollama_rs::{
    error::OllamaError,
    generation::{
        chat::{request::ChatMessageRequest, ChatMessage, ChatMessageResponse},
        options::GenerationOptions,
        parameters::{FormatType, JsonStructure},
        tools::ToolInfo,
    },
    Ollama,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
// use serde_json::Value;
use std::borrow::Cow;
use tracing::{error, info};
use url::Url;

const DEFAULT_MODEL_NAME: &str = "llama3.2";
const DEFAULT_ENDPOINT: &str = "http://localhost:11434";
const DEFAULT_MESSAGES_NUMBER_LIMIT: usize = 10;

/// Enumerates the types of errors that can occur in LLM
#[derive(thiserror::Error, Debug)]
pub enum ChatError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Tool call error: {0}")]
    ToolCall(String),
    #[error("JSON error: {0}")]
    JsonError(serde_json::Error),
    #[error("Reqwest error: {0}")]
    ReqwestError(reqwest::Error),
    #[error("Ollama internal error: {0}")]
    OllamaInternal(String),
    #[error("Ollama other error: {0}")]
    OllamaOther(String),
    #[error("Ollama not initialized")]
    OllamaNotInitialized,
}

impl From<OllamaError> for ChatError {
    fn from(err: OllamaError) -> Self {
        match err {
            OllamaError::ToolCallError(e) => ChatError::ToolCall(e.to_string()),
            OllamaError::JsonError(e) => ChatError::JsonError(e),
            OllamaError::ReqwestError(e) => ChatError::ReqwestError(e),
            OllamaError::InternalError(e) => ChatError::OllamaInternal(e.message),
            OllamaError::Other(e) => ChatError::OllamaOther(e),
        }
    }
}

impl ActorError for ChatError {}

#[derive(bon::Builder, Debug, Clone, Serialize, Deserialize)]
pub struct Chat {
    #[builder(default = DEFAULT_MODEL_NAME.into())]
    pub model: Cow<'static, str>,
    pub generation_options: Option<GenerationOptions>,
    #[builder(default = Url::parse(DEFAULT_ENDPOINT).unwrap())]
    pub endpoint: Url,
    #[builder(default = DEFAULT_MESSAGES_NUMBER_LIMIT)]
    pub messages_number_limit: usize,
    #[builder(default)]
    pub history: Vec<ChatMessage>,
    #[serde(skip)]
    #[builder(default)]
    ollama: Ollama,
}

impl Default for Chat {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL_NAME.into(),
            generation_options: None,
            endpoint: Url::parse(DEFAULT_ENDPOINT).unwrap(),
            messages_number_limit: DEFAULT_MESSAGES_NUMBER_LIMIT,
            history: Vec::new(),
            ollama: Ollama::default(),
        }
    }
}

#[derive(utoipa::ToSchema, Debug, Clone, Serialize, Deserialize)]
#[schema(title = "Schema")]
pub struct Schema {
    #[serde(flatten)]
    #[schema(value_type = schema::Object)]
    schema: schemars::schema::RootSchema,
}

impl Schema {
    pub fn new<T: JsonSchema>() -> Self {
        // Configure schema settings to inline subschemas (required for Ollama)
        let mut settings = schemars::gen::SchemaSettings::draft07();
        settings.inline_subschemas = true;
        let generator = settings.into_generator();
        let schema = generator.into_root_schema_for::<T>();
        Self { schema }
    }

    pub fn schema_json(&self) -> String {
        serde_json::to_string(&self.schema).unwrap()
    }
}

#[derive(bon::Builder, Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessages {
    pub messages: Vec<ChatMessage>,
    #[builder(default)]
    pub restart: bool,
    #[builder(default)]
    pub persist: bool,
    #[builder(default)]
    pub stream: bool,
    pub format: Option<Schema>,
    pub tools: Option<Vec<ToolInfo>>,
}

impl Message<ChatMessages> for Chat {
    type Response = ChatMessageResponse;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, request: &ChatMessages) -> Result<(), ChatError> {
        // Get stream flag, may be changed by tools
        let mut stream = request.stream;

        if request.restart {
            self.history.clear();
        }

        // Add new messages to history
        for message in &request.messages {
            self.history.push(message.clone());
            if self.history.len() > self.messages_number_limit {
                self.history.drain(..self.history.len() - self.messages_number_limit);
            }
        }

        // Prepare chat request
        let mut chat_message_request = ChatMessageRequest::new(self.model.to_string(), self.history.clone());

        // Add tools
        if let Some(tools) = &request.tools {
            chat_message_request.tools = tools.clone();
            stream = false;
        }

        // Add generation options
        if let Some(generation_options) = &self.generation_options {
            chat_message_request = chat_message_request.options(generation_options.clone());
        }

        // Add format
        if let Some(format) = &request.format {
            chat_message_request = chat_message_request
                .format(FormatType::StructuredJson(JsonStructure::from_schema(format.schema.clone())));
        }

        // // Save chat request to debug file
        // let debug_path = std::path::Path::new(".output/chat_request.json");
        // if let Some(parent) = debug_path.parent() {
        //     if !parent.exists() {
        //         if let Err(e) = std::fs::create_dir_all(parent) {
        //             error!("Failed to create debug output directory: {}", e);
        //         }
        //     }
        // }
        // if let Err(e) = std::fs::write(
        //     debug_path,
        //     serde_json::to_string_pretty(&chat_message_request).unwrap_or_default()
        // ) {
        //     error!("Failed to write chat request debug file: {}", e);
        // }

        if stream {
            // Get streaming response from Ollama
            let mut stream = self.ollama.send_chat_messages_stream(chat_message_request).await?;
            let mut accumulated_content = String::new();

            // Stream responses back to caller
            while let Some(response) = stream.next().await {
                match response {
                    Ok(chunk) => {
                        // Send chunk through actor's reply mechanism
                        ctx.reply(chunk.clone()).await?;

                        // Accumulate message content
                        accumulated_content.push_str(&chunk.message.content);

                        // If this is the final message, add the complete message to history
                        if chunk.done {
                            if !accumulated_content.is_empty() {
                                self.history.push(ChatMessage::assistant(accumulated_content.clone()));
                            }

                            // Persist if requested
                            if request.persist {
                                self.history = self
                                    .history
                                    .iter()
                                    .filter(|msg| msg.role != ollama_rs::generation::chat::MessageRole::System)
                                    .cloned()
                                    .collect();
                                self.save(ctx).await?;
                            }
                        }
                    }
                    Err(_) => {
                        error!("Error in chat stream");
                        break;
                    }
                }
            }
        } else {
            // Send the messages to the ollama client
            let result = self.ollama.send_chat_messages(chat_message_request).await?;

            // Add the response message to the history only if its an assistant message
            if result.message.role == ollama_rs::generation::chat::MessageRole::Assistant {
                self.history.push(result.message.clone());
            }

            if request.persist {
                // Filter out system messages before saving
                self.history = self
                    .history
                    .iter()
                    .filter(|msg| msg.role != ollama_rs::generation::chat::MessageRole::System)
                    .cloned()
                    .collect();
                self.save(ctx).await?;
            }

            ctx.reply(result).await?;
        }

        Ok(())
    }
}

impl Actor for Chat {
    type Error = ChatError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), ChatError> {
        info!("{} Started", ctx.id());

        self.init(ctx).await?;

        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(chat_messages) = frame.is::<ChatMessages>() {
                let response = self.reply(ctx, &chat_messages, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            }
        }
        info!("{} Finished", ctx.id());
        Ok(())
    }
}

impl Chat {
    pub async fn init(&mut self, _ctx: &mut ActorContext<Self>) -> Result<(), ChatError> {
        self.ollama = Ollama::from_url(self.endpoint.clone());
        Ok(())
    }
}
