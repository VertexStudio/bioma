use bioma_actor::prelude::*;
use ollama_rs::{
    error::OllamaError,
    generation::{
        chat::{request::ChatMessageRequest, ChatMessage, ChatMessageResponse},
        options::GenerationOptions,
    },
    Ollama,
};
use serde::{Deserialize, Serialize};
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
    #[error("Ollama error: {0}")]
    Ollama(#[from] OllamaError),
    #[error("Ollama not initialized")]
    OllamaNotInitialized,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessages {
    pub messages: Vec<ChatMessage>,
    pub restart: bool,
    pub persist: bool,
}

impl Message<ChatMessages> for Chat {
    type Response = ChatMessageResponse;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, messages: &ChatMessages) -> Result<(), ChatError> {
        if messages.restart {
            self.history.clear();
        }

        for message in messages.messages.iter() {
            // Add the message to the history
            self.history.push(message.clone());

            // Truncate history if it exceeds the limit
            if self.history.len() > self.messages_number_limit {
                self.history.drain(..self.history.len() - self.messages_number_limit);
            }
        }

        let mut chat_message_request = ChatMessageRequest::new(self.model.to_string(), self.history.clone());
        if let Some(generation_options) = &self.generation_options {
            chat_message_request = chat_message_request.options(generation_options.clone());
        }

        // Send the messages to the ollama client
        let result = self.ollama.send_chat_messages(chat_message_request).await?;

        // Add the assistant's message to the history
        if let Some(message) = &result.message {
            self.history.push(ChatMessage::assistant(message.content.clone()));
        }

        if messages.persist {
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
