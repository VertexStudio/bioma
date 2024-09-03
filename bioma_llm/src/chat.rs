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
use tracing::info;

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
    pub generation_options: GenerationOptions,
    pub messages_number_limit: usize,
    pub history: Vec<ChatMessage>,
    #[serde(skip)]
    pub ollama: Option<Ollama>,
}

impl Message<ChatMessage> for Chat {
    type Response = ChatMessageResponse;

    fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        message: &ChatMessage,
    ) -> impl Future<Output = Result<ChatMessageResponse, ChatError>> {
        async move {
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

            // Send the messages to the ollama client
            let result = ollama
                .send_chat_messages(ChatMessageRequest::new(self.model_name.clone(), self.history.clone()))
                .await?;

            // Add the assistant's message to the history
            if let Some(message) = &result.message {
                self.history.push(ChatMessage::assistant(message.content.clone()));
            }

            Ok(result)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SystemMessage {
    Exit,
}

impl Message<SystemMessage> for Chat {
    type Response = ();

    fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        _message: &SystemMessage,
    ) -> impl Future<Output = Result<(), ChatError>> {
        async move { Ok(()) }
    }
}

impl Actor for Chat {
    type Error = ChatError;

    fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), ChatError>> {
        async move {
            info!("{} Started", ctx.id());

            self.ollama = Some(Ollama::default());

            let mut stream = ctx.recv().await?;
            while let Some(Ok(frame)) = stream.next().await {
                if let Some(chat_message) = frame.is::<ChatMessage>() {
                    self.reply(ctx, &chat_message, &frame).await?;
                } else if let Some(sys_msg) = frame.is::<SystemMessage>() {
                    self.reply(ctx, &sys_msg, &frame).await?;
                    match sys_msg {
                        SystemMessage::Exit => break,
                    }
                }
            }
            info!("{} Finished", ctx.id());
            Ok(())
        }
    }
}
