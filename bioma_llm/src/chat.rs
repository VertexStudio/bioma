use bioma_actor::prelude::*;
use lazy_static::lazy_static;
use mistralrs::{ChatCompletionResponse, GgufModelBuilder, Model, TextMessageRole, TextMessages};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tracing::{error, info};

const DEFAULT_MODEL_ID: &str = "bartowski/Qwen2.5-Coder-3B-Instruct-GGUF";
const DEFAULT_MODEL_FILES: [&str; 1] = ["Qwen2.5-Coder-3B-Instruct-Q6_K.gguf"];
const DEFAULT_TOK_MODEL_ID: &str = "Qwen/Qwen2.5-Coder-3B-Instruct";
const DEFAULT_MESSAGES_NUMBER_LIMIT: usize = 10;

lazy_static! {
    static ref SHARED_MODEL: Arc<Mutex<Option<Arc<SharedModel>>>> = Arc::new(Mutex::new(None));
}

struct SharedModel {
    chat_tx: mpsc::Sender<SharedModelRequest>,
}

impl std::fmt::Debug for SharedModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SharedModel")
    }
}

struct SharedModelRequest {
    response_tx: oneshot::Sender<Result<ChatCompletionResponse, anyhow::Error>>,
    messages: TextMessages,
}

#[derive(thiserror::Error, Debug)]
pub enum ChatError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Mistral error: {0}")]
    Mistral(#[from] anyhow::Error),
    #[error("Model not initialized")]
    ModelNotInitialized,
    #[error("Error sending chat request: {0}")]
    SendChat(#[from] mpsc::error::SendError<SharedModelRequest>),
    #[error("Error receiving chat response: {0}")]
    RecvChat(#[from] oneshot::error::RecvError),
}

impl ActorError for ChatError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: TextMessageRole,
    pub content: Cow<'static, str>,
}

impl ChatMessage {
    pub fn system(content: impl Into<Cow<'static, str>>) -> Self {
        Self { role: TextMessageRole::System, content: content.into() }
    }

    pub fn user(content: impl Into<Cow<'static, str>>) -> Self {
        Self { role: TextMessageRole::User, content: content.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub restart: bool,
    pub persist: bool,
}

#[derive(bon::Builder, Debug, Serialize, Deserialize)]
pub struct Chat {
    #[builder(default = DEFAULT_MODEL_ID.into())]
    pub model_id: Cow<'static, str>,
    #[builder(default = DEFAULT_MODEL_FILES.iter().map(|&s| s.into()).collect())]
    pub model_files: Vec<Cow<'static, str>>,
    #[builder(default = DEFAULT_TOK_MODEL_ID.into())]
    pub tok_model_id: Cow<'static, str>,
    #[builder(default)]
    pub history: Vec<ChatMessage>,
    #[builder(default = DEFAULT_MESSAGES_NUMBER_LIMIT)]
    pub messages_number_limit: usize,
    #[serde(skip)]
    chat_tx: Option<mpsc::Sender<SharedModelRequest>>,
    #[serde(skip)]
    shared_model: Option<Arc<SharedModel>>,
}

impl Default for Chat {
    fn default() -> Self {
        Self {
            model_id: DEFAULT_MODEL_ID.into(),
            model_files: DEFAULT_MODEL_FILES.iter().map(|&s| s.into()).collect(),
            tok_model_id: DEFAULT_TOK_MODEL_ID.into(),
            history: Vec::new(),
            messages_number_limit: DEFAULT_MESSAGES_NUMBER_LIMIT,
            chat_tx: None,
            shared_model: None,
        }
    }
}

impl Message<ChatRequest> for Chat {
    type Response = ChatCompletionResponse;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        request: &ChatRequest,
    ) -> Result<ChatCompletionResponse, ChatError> {
        let Some(chat_tx) = self.chat_tx.as_ref() else {
            return Err(ChatError::ModelNotInitialized);
        };

        if request.restart {
            self.history.clear();
        }

        for message in request.messages.iter() {
            self.history.push(message.clone());
            if self.history.len() > self.messages_number_limit {
                self.history.drain(..self.history.len() - self.messages_number_limit);
            }
        }

        let mut mistral_request = TextMessages::new();
        for message in self.history.iter() {
            mistral_request = mistral_request.add_message(message.role.clone(), message.content.clone());
        }

        let (tx, rx) = oneshot::channel();
        chat_tx.send(SharedModelRequest { response_tx: tx, messages: mistral_request }).await?;
        let response = rx.await??;

        if let Some(message) = &response.choices.first().and_then(|choice| choice.message.content.clone()) {
            self.history.push(ChatMessage { role: TextMessageRole::Assistant, content: message.clone().into() });
        }

        if request.persist {
            self.history = self.history
                .iter()
                .filter(|msg| msg.role != TextMessageRole::System)
                .cloned()
                .collect();
            self.save(ctx).await?;
        }

        Ok(response)
    }
}

impl Actor for Chat {
    type Error = ChatError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), ChatError> {
        info!("{} Started", ctx.id());

        let shared_model = {
            let mut guard = SHARED_MODEL.lock().await;
            if let Some(model) = guard.clone() {
                model
            } else {
                let ctx_id = ctx.id().clone();
                let model_id = self.model_id.clone();
                let model_files = self.model_files.clone();
                let tok_model_id = self.tok_model_id.clone();
                
                let (chat_tx, chat_rx) = mpsc::channel::<SharedModelRequest>(100);

                // Spawn a blocking task that handles both initialization and message processing
                tokio::task::spawn_blocking(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("Failed to build runtime");

                    runtime.block_on(async move {
                        // Initialize the model
                        let model = GgufModelBuilder::new(model_id.to_string(), model_files.iter().map(|s| s.to_string()).collect())
                            .with_tok_model_id(tok_model_id.to_string())
                            // .with_logging()
                            .build()
                            .await
                            .expect("Failed to initialize model");

                        info!("{} model initialized", ctx_id);

                        let mut chat_rx = chat_rx;
                        while let Some(request) = chat_rx.recv().await {
                            let start = std::time::Instant::now();

                            match model.send_chat_request(request.messages).await {
                                Ok(response) => {
                                    info!("Generated chat response in {:?}", start.elapsed());
                                    let _ = request.response_tx.send(Ok(response));
                                }
                                Err(err) => {
                                    error!("Failed to generate chat response: {}", err);
                                    let _ = request.response_tx.send(Err(err));
                                }
                            }
                        }

                        info!("{} chat model finished", ctx_id);
                    });
                });

                let shared_model = Arc::new(SharedModel { chat_tx });
                *guard = Some(shared_model.clone());
                shared_model
            }
        };

        self.shared_model = Some(shared_model.clone());
        self.chat_tx = Some(shared_model.chat_tx.clone());

        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(chat_request) = frame.is::<ChatRequest>() {
                let response = self.reply(ctx, &chat_request, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            }
        }

        info!("{} Finished", ctx.id());
        Ok(())
    }
}