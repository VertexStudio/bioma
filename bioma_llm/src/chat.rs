use bioma_actor::prelude::*;
use futures::Stream;
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
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_stream::StreamExt;
use tracing::{error, info};
use url::Url;

// Add a variant to OllamaError to handle unknown errors
// If OllamaError is part of an external crate you may need to create your own error type
// and unify error types differently. If you can edit OllamaError:
#[derive(thiserror::Error, Debug)]
pub enum ChatError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Ollama error: {0}")]
    Ollama(#[from] OllamaError),
    #[error("Ollama not initialized")]
    OllamaNotInitialized,
    #[error("Stream not started")]
    StreamNotStarted,
}

impl ActorError for ChatError {}

// Add an Unknown variant in OllamaError if not present
// If you cannot modify OllamaError, create your own error type and wrap OllamaError
#[derive(thiserror::Error, Debug)]
pub enum MyOllamaError {
    #[error("Unknown Ollama error: {0}")]
    Unknown(String),
    #[error("Ollama error: {0}")]
    Original(#[from] OllamaError),
}

type ChatStream = Pin<Box<dyn Stream<Item = Result<ChatMessageResponse, MyOllamaError>> + Send>>;

#[derive(bon::Builder, Serialize, Deserialize)]
pub struct Chat {
    #[builder(default = "llama3.2".into())]
    pub model: Cow<'static, str>,
    pub generation_options: Option<GenerationOptions>,
    #[builder(default = Url::parse("http://localhost:11434").unwrap())]
    pub endpoint: Url,
    #[builder(default = 10)]
    pub messages_number_limit: usize,
    #[builder(default)]
    pub history: Vec<ChatMessage>,
    #[serde(skip)]
    #[builder(default)]
    ollama: Ollama,
    #[serde(skip)]
    #[builder(default)]
    stream: Arc<Mutex<Option<ChatStream>>>,
}

impl std::fmt::Debug for Chat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Chat")
            .field("model", &self.model)
            .field("generation_options", &self.generation_options)
            .field("endpoint", &self.endpoint)
            .field("messages_number_limit", &self.messages_number_limit)
            .field("history", &self.history)
            .field("ollama", &self.ollama)
            .field("stream", &"<stream>")
            .finish()
    }
}

impl Default for Chat {
    fn default() -> Self {
        Self {
            model: "llama3.2".into(),
            generation_options: None,
            endpoint: Url::parse("http://localhost:11434").unwrap(),
            messages_number_limit: 10,
            history: Vec::new(),
            ollama: Ollama::default(),
            stream: Arc::new(Mutex::new(None)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatStreamStart {
    pub messages: Vec<ChatMessage>,
    pub restart: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatStreamNext {
    pub persist: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatStreamChunk {
    pub response: Option<ChatMessageResponse>,
    pub done: bool,
}

impl Message<ChatStreamStart> for Chat {
    type Response = ChatStreamChunk;

    async fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        start: &ChatStreamStart,
    ) -> Result<ChatStreamChunk, ChatError> {
        if start.restart {
            self.history.clear();
        }

        for message in &start.messages {
            self.history.push(message.clone());
            if self.history.len() > self.messages_number_limit {
                self.history.drain(..self.history.len() - self.messages_number_limit);
            }
        }

        let mut chat_message_request = ChatMessageRequest::new(self.model.to_string(), self.history.clone());
        if let Some(generation_options) = &self.generation_options {
            chat_message_request = chat_message_request.options(generation_options.clone());
        }

        // This presumably returns Stream<Item=Result<ChatMessageResponse, ()>>
        let raw_stream = self.ollama.send_chat_messages_stream(chat_message_request).await?;

        // Map the () error to MyOllamaError::Unknown:
        let mapped_stream = raw_stream.map(|res: Result<ChatMessageResponse, ()>| {
            res.map_err(|_| MyOllamaError::Unknown("Unknown error from Ollama".into()))
        });

        let mut lock = self.stream.lock().await;
        *lock = Some(Box::pin(mapped_stream));

        Ok(ChatStreamChunk { response: None, done: false })
    }
}

impl Message<ChatStreamNext> for Chat {
    type Response = ChatStreamChunk;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        next: &ChatStreamNext,
    ) -> Result<ChatStreamChunk, ChatError> {
        let mut lock = self.stream.lock().await;
        let stream = match lock.as_mut() {
            Some(s) => s,
            None => return Err(ChatError::StreamNotStarted),
        };

        match stream.next().await {
            Some(Ok(chunk)) => {
                println!("Chunk: {:?}", chunk);
                let done = chunk.done;

                // Accumulate message content when it's available
                if let Some(message) = &chunk.message {
                    self.history.push(ChatMessage::assistant(message.content.clone()));
                    if done {
                        if next.persist {
                            self.history = self
                                .history
                                .iter()
                                .filter(|msg| msg.role != ollama_rs::generation::chat::MessageRole::System)
                                .cloned()
                                .collect();
                            self.save(ctx).await?;
                        }
                        *lock = None;
                    }
                }

                Ok(ChatStreamChunk { response: Some(chunk), done })
            }
            Some(Err(e)) => {
                error!("Error fetching stream chunk: {:?}", e);
                // Convert MyOllamaError back to ChatError if needed
                // If e is MyOllamaError::Original(err), convert to ChatError::Ollama
                // If MyOllamaError::Unknown(...), you can handle accordingly.
                match e {
                    MyOllamaError::Original(err) => Err(ChatError::Ollama(err)),
                    MyOllamaError::Unknown(msg) => Err(ChatError::Ollama(OllamaError::from(msg))),
                }
            }
            None => {
                // Stream ended
                println!("Stream ended");
                *lock = None;
                Ok(ChatStreamChunk { response: None, done: true })
            }
        }
    }
}

impl Actor for Chat {
    type Error = ChatError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), ChatError> {
        info!("{} Started", ctx.id());
        self.init(ctx).await?;

        let mut frames = ctx.recv().await?;
        while let Some(Ok(frame)) = frames.next().await {
            if let Some(start) = frame.is::<ChatStreamStart>() {
                let response = self.reply(ctx, &start, &frame).await;
                if let Err(e) = response {
                    error!("Error: {:?}", e);
                }
            }

            if let Some(next) = frame.is::<ChatStreamNext>() {
                let response = self.reply(ctx, &next, &frame).await;
                if let Err(e) = response {
                    error!("Error: {:?}", e);
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
