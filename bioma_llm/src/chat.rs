use bioma_actor::prelude::*;
use derive_more::Deref;
use lazy_static::lazy_static;
use mistralrs::{
    paged_attn_supported, ChatCompletionResponse, DrySamplingParams, GgufModelBuilder, MemoryGpuConfig,
    PagedAttentionConfig, RequestBuilder, SamplingParams, TextMessageRole, TextMessages,
};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::sync::{Arc, Weak};
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{error, info};

const DEFAULT_MODEL_ID: &str = "bartowski/Qwen2.5-Coder-3B-Instruct-GGUF";
const DEFAULT_MODEL_FILES: [&str; 1] = ["Qwen2.5-Coder-3B-Instruct-Q4_K_M.gguf"];
const DEFAULT_MODEL_CONTEXT_SIZE: usize = 32768;
const DEFAULT_MESSAGES_NUMBER_LIMIT: usize = 10;

lazy_static! {
    static ref SHARED_MODEL: Arc<Mutex<Weak<SharedModel>>> = Arc::new(Mutex::new(Weak::new()));
}

struct SharedModel {
    chat_tx: mpsc::Sender<ChatTextRequest>,
}

pub struct ChatTextRequest {
    response_tx: oneshot::Sender<Result<ChatCompletionResponse, anyhow::Error>>,
    messages: TextMessages,
    sampling_params: Option<SamplingParams>,
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

#[derive(bon::Builder, Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub restart: bool,
    pub persist: bool,
    pub sampling_params: Option<SamplingParams>,
}

#[derive(bon::Builder, Debug, Serialize, Deserialize)]
pub struct Chat {
    #[builder(default = DEFAULT_MODEL_ID.into())]
    pub model_id: Cow<'static, str>,
    #[builder(default = DEFAULT_MODEL_FILES.iter().map(|&s| s.into()).collect())]
    pub model_files: Vec<Cow<'static, str>>,
    #[builder(default = DEFAULT_MODEL_CONTEXT_SIZE)]
    pub model_context_size: usize,
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
            model_context_size: DEFAULT_MODEL_CONTEXT_SIZE,
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
        chat_tx
            .send(ChatTextRequest {
                response_tx: tx,
                messages: mistral_request,
                sampling_params: request.sampling_params.clone(),
            })
            .await?;
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
                let model_context_size = self.model_context_size;
                let (chat_tx, mut chat_rx) = mpsc::channel::<ChatTextRequest>(100);

                let (shared_model_tx, shared_model_rx) = oneshot::channel();
                std::thread::spawn(move || {
                    let runtime = tokio::runtime::Runtime::new().unwrap();
                    runtime.block_on(async move {
                        let mut model = GgufModelBuilder::new(
                            model_id.to_string(),
                            model_files.iter().map(|s| s.to_string()).collect(),
                        )
                        // .with_tok_model_id(tok_model_id.to_string())
                        // .with_token_source(TokenSource::CacheToken)
                        .with_max_num_seqs(32)
                        .with_prefix_cache_n(Some(16));

                        if paged_attn_supported() {
                            model = model
                                .with_paged_attn(|| {
                                    Ok(PagedAttentionConfig::new(
                                        None,
                                        512,
                                        MemoryGpuConfig::ContextSize(model_context_size),
                                    )?)
                                })
                                .unwrap();
                        }

                        let model = model.build().await.expect("Failed to initialize model");

                        {
                            let shared_model = Arc::new(SharedModel { chat_tx });
                            *weak_ref = Arc::downgrade(&shared_model);
                            let _ = shared_model_tx.send(shared_model);
                        }

                        info!("{} Model initialized", ctx_id);

                        // Process messages
                        while let Some(request) = chat_rx.recv().await {
                            let sampling_params = match request.sampling_params {
                                Some(params) => params,
                                None => SamplingParams {
                                    temperature: Some(0.8),
                                    top_k: Some(32),
                                    top_p: Some(0.1),
                                    min_p: Some(0.05),
                                    top_n_logprobs: 0,
                                    frequency_penalty: Some(1.5),
                                    presence_penalty: Some(0.1),
                                    stop_toks: None,
                                    max_len: Some(4096),
                                    logits_bias: None,
                                    n_choices: 1,
                                    dry_params: Some(DrySamplingParams {
                                        sequence_breakers: vec![
                                            "\n".to_string(),
                                            ":".to_string(),
                                            "\"".to_string(),
                                            "*".to_string(),
                                        ],
                                        multiplier: 0.0,
                                        base: 1.75,
                                        allowed_length: 2,
                                    }),
                                },
                            };

                            info!("sampling_params: {:#?}", sampling_params);

                            info!("request.messages: {:#?}", request.messages);

                            let request_builder = RequestBuilder::from(request.messages).set_sampling(sampling_params);

                            let start = std::time::Instant::now();

                            match model.send_chat_request(request_builder).await {
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
