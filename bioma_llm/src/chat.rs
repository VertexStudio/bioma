use bioma_actor::prelude::*;
use derive_more::Deref;
use lazy_static::lazy_static;
use mistralrs::{ChatCompletionResponse, GgufModelBuilder, Model, TextMessageRole, TextMessages};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::sync::{Arc, Weak};
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{error, info};

const DEFAULT_MODEL_ID: &str = "bartowski/Qwen2.5-Coder-3B-Instruct-GGUF";
const DEFAULT_MODEL_FILES: [&str; 1] = ["Qwen2.5-Coder-3B-Instruct-Q6_K.gguf"];
const DEFAULT_TOK_MODEL_ID: &str = "Qwen/Qwen2.5-Coder-3B-Instruct";
const DEFAULT_MESSAGES_NUMBER_LIMIT: usize = 10;

lazy_static! {
    static ref SHARED_MODEL: Arc<Mutex<Weak<SharedModel>>> = Arc::new(Mutex::new(Weak::new()));
}

struct SharedModel {
    chat_tx: mpsc::Sender<ChatTextRequest>,
    // model: Model,
}

pub struct ChatTextRequest {
    response_tx: oneshot::Sender<Result<ChatCompletionResponse, anyhow::Error>>,
    messages: TextMessages,
}

#[derive(Deref)]
struct StrongSharedModel(Arc<SharedModel>);

impl std::fmt::Debug for StrongSharedModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StrongSharedModel")
    }
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
    SendChatText(#[from] mpsc::error::SendError<ChatTextRequest>),
    #[error("Error receiving chat response: {0}")]
    RecvChatText(#[from] oneshot::error::RecvError),
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
    chat_tx: Option<mpsc::Sender<ChatTextRequest>>,
    #[serde(skip)]
    shared_model: Option<StrongSharedModel>,
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
        info!("{} Handling chat request 0", ctx.id());
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

        info!("{} Handling chat request 1", ctx.id());

        let mut mistral_request = TextMessages::new();
        for message in self.history.iter() {
            mistral_request = mistral_request.add_message(message.role.clone(), message.content.clone());
        }

        info!("{} Handling chat request 2", ctx.id());

        let (tx, rx) = oneshot::channel();
        chat_tx.send(ChatTextRequest { response_tx: tx, messages: mistral_request }).await?;
        let response = rx.await??;

        info!("{} Handling chat request 3", ctx.id());

        if let Some(message) = &response.choices.first().and_then(|choice| choice.message.content.clone()) {
            self.history.push(ChatMessage { role: TextMessageRole::Assistant, content: message.clone().into() });
        }

        info!("{} Handling chat request 4", ctx.id());

        if request.persist {
            self.history = self.history.iter().filter(|msg| msg.role != TextMessageRole::System).cloned().collect();
            self.save(ctx).await?;
        }

        info!("{} Handling chat request 5", ctx.id());

        Ok(response)
    }
}

impl Actor for Chat {
    type Error = ChatError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), ChatError> {
        info!("{} Started", ctx.id());

        let shared_model = {
            let mut weak_ref = SHARED_MODEL.lock().await;
            if let Some(strong_ref) = weak_ref.upgrade() {
                // Return the existing shared model
                info!("{} Sharing model: {}", ctx.id(), self.model_id);
                Some(strong_ref)
            } else {
                info!("{} Loading model: {}", ctx.id(), self.model_id);

                let ctx_id = ctx.id().clone();
                let model_id = self.model_id.clone();
                let model_files = self.model_files.clone();
                let tok_model_id = self.tok_model_id.clone();

                let (chat_tx, mut chat_rx) = mpsc::channel::<ChatTextRequest>(100);

                let (shared_model_tx, shared_model_rx) = oneshot::channel();
                std::thread::spawn(move || {
                    let runtime = tokio::runtime::Runtime::new().unwrap();
                    runtime.block_on(async move {
                        let model = GgufModelBuilder::new(
                            model_id.to_string(),
                            model_files.iter().map(|s| s.to_string()).collect(),
                        )
                        .with_tok_model_id(tok_model_id.to_string())
                        .build()
                        .await
                        .expect("Failed to initialize model");

                        {
                            let shared_model = Arc::new(SharedModel { chat_tx });
                            *weak_ref = Arc::downgrade(&shared_model);
                            let _ = shared_model_tx.send(shared_model);
                        }

                        info!("{} Model initialized", ctx_id);

                        // Process messages
                        while let Some(request) = chat_rx.recv().await {
                            let start = std::time::Instant::now();

                            // let model = SHARED_MODEL.lock().await.upgrade();
                            // let Some(model) = model else {
                            //     error!("{} Model dropped", ctx_id);
                            //     return;
                            // };
                            // let model = &model.model;

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

                        info!("{} Model finished", ctx_id);
                    });
                });

                let shared_model = shared_model_rx.await.unwrap();
                Some(shared_model)
            }
        };

        self.shared_model = shared_model.map(StrongSharedModel);
        self.chat_tx = self.shared_model.as_ref().map(|sm| sm.chat_tx.clone());

        info!("{} Ready", ctx.id());

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
