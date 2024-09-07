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
use tracing::{error, info};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chat {
    pub model_name: String,
    pub generation_options: Option<GenerationOptions>,
    pub messages_number_limit: usize,
    pub history: Vec<ChatMessage>,
    #[serde(skip)]
    pub ollama: Option<Ollama>,
}

impl Message<ChatMessage> for Chat {
    type Response = ChatMessageResponse;

    async fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        message: &ChatMessage,
    ) -> Result<ChatMessageResponse, ChatError> {
        // Check if the ollama client is initialized
        let Some(ollama) = &self.ollama else {
            return Err(ChatError::OllamaNotInitialized);
        };

        // Add the user's message to the history
        self.history.push(message.clone());

        // Truncate history if it exceeds the limit
        if self.history.len() > self.messages_number_limit {
            self.history.drain(..self.history.len() - self.messages_number_limit);
        }

        let mut chat_message_request = ChatMessageRequest::new(self.model_name.clone(), self.history.clone());
        if let Some(generation_options) = &self.generation_options {
            chat_message_request = chat_message_request.options(generation_options.clone());
        }

        // Send the messages to the ollama client
        let result = ollama.send_chat_messages(chat_message_request).await?;

        // Add the assistant's message to the history
        if let Some(message) = &result.message {
            self.history.push(ChatMessage::assistant(message.content.clone()));
        }

        Ok(result)
    }
}

impl Actor for Chat {
    type Error = ChatError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), ChatError> {
        info!("{} Started", ctx.id());

        self.ollama = Some(Ollama::default());

        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(chat_message) = frame.is::<ChatMessage>() {
                let response = self.reply(ctx, &chat_message, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            }
        }
        info!("{} Finished", ctx.id());
        Ok(())
    }
}
