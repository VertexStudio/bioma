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
use std::pin::Pin;
use tokio_stream::StreamExt;
use tracing::{error, info, warn};
use url::Url;

const DEFAULT_MODEL_NAME: &str = "llama3.2";
const DEFAULT_ENDPOINT: &str = "http://localhost:11434";
const DEFAULT_MESSAGES_NUMBER_LIMIT: usize = 10;

/// Enumerates the types of errors that can occur
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

#[derive(bon::Builder, Serialize, Deserialize)]
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
    #[serde(skip)]
    stream: Option<Pin<Box<dyn futures::Stream<Item = Result<ChatMessageResponse, OllamaError>> + Send + Sync>>>,
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
            stream: None,
        }
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatStreamStart {
    pub messages: Vec<ChatMessage>,
    pub restart: bool,
    pub persist: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatStreamNext;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatStreamChunk {
    pub response: Option<ChatMessageResponse>,
    pub done: bool,
}

impl Message<ChatStreamStart> for Chat {
    type Response = ChatStreamChunk;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
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

        // Start streaming
        let stream = self.ollama.send_chat_messages_stream(chat_message_request).await?;
        self.stream = Some(Box::pin(stream));

        // We do not consume any chunk yet, we just return a status that we're ready
        Ok(ChatStreamChunk { response: None, done: false })
    }
}

impl Message<ChatStreamNext> for Chat {
    type Response = ChatStreamChunk;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        _next: &ChatStreamNext,
    ) -> Result<ChatStreamChunk, ChatError> {
        let stream = match &mut self.stream {
            Some(s) => s,
            None => return Err(ChatError::StreamNotStarted),
        };

        match stream.next().await {
            Some(Ok(chunk)) => {
                // We got a new chunk
                let done = chunk.done;
                if done {
                    // If done, add to history
                    if let Some(message) = &chunk.message {
                        self.history.push(ChatMessage::assistant(message.content.clone()));
                    }
                    // You could choose to clear the stream here or leave it
                    self.stream = None;
                }
                Ok(ChatStreamChunk { response: Some(chunk), done })
            }
            Some(Err(e)) => {
                error!("Error fetching stream chunk: {:?}", e);
                Err(ChatError::Ollama(e))
            }
            None => {
                // No more chunks
                self.stream = None;
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
            // Handle stream start
            if let Some(start) = frame.is::<ChatStreamStart>() {
                let response = self.reply(ctx, &start, &frame).await;
                if let Err(e) = response {
                    error!("Error: {:?}", e);
                }
            }

            // Handle next chunk request
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
