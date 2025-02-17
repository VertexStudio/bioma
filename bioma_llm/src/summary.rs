use bioma_actor::prelude::*;
use ollama_rs::generation::chat::ChatMessage;
use serde::{Deserialize, Serialize};
use tracing::{debug, error};

use crate::chat::{Chat, ChatError, ChatMessages};

#[derive(thiserror::Error, Debug)]
pub enum SummaryError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Chat error: {0}")]
    Chat(#[from] ChatError),
    #[error("Chat actor not initialized")]
    ChatActorNotInitialized,
}

impl ActorError for SummaryError {}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummarizeText {
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummaryResponse {
    pub summary: String,
}

#[derive(bon::Builder, Debug, Serialize, Deserialize)]
pub struct Summary {
    pub chat: Chat,
    chat_id: Option<ActorId>,
    #[serde(skip)]
    #[builder(skip)]
    chat_handle: Option<tokio::task::JoinHandle<()>>,
}

impl Default for Summary {
    fn default() -> Self {
        Self { chat: Chat::default(), chat_id: None, chat_handle: None }
    }
}

impl Clone for Summary {
    fn clone(&self) -> Self {
        Self { chat: self.chat.clone(), chat_id: self.chat_id.clone(), chat_handle: None }
    }
}

impl Actor for Summary {
    type Error = SummaryError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        self.init(ctx).await?;

        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(input) = frame.is::<SummarizeText>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            }
        }

        Ok(())
    }
}

impl Message<SummarizeText> for Summary {
    type Response = SummaryResponse;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, message: &SummarizeText) -> Result<(), Self::Error> {
        let Some(chat_id) = &self.chat_id else {
            return Err(SummaryError::ChatActorNotInitialized);
        };

        // Create a prompt for summarization
        let prompt = format!(
            "Provide a concise summary of the following. Focus on the key points and main ideas:\n\n{}",
            message.text
        );

        // Send message to chat actor
        let response = ctx
            .send_and_wait_reply::<Chat, ChatMessages>(
                ChatMessages::builder().messages(vec![ChatMessage::user(prompt)]).build(),
                chat_id,
                SendOptions::builder().timeout(std::time::Duration::from_secs(300)).build(),
            )
            .await?;

        // Extract summary from chat response
        ctx.reply(SummaryResponse { summary: response.message.content }).await?;
        Ok(())
    }
}

impl Summary {
    pub async fn init(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), SummaryError> {
        // Used to namespace child actors
        let self_id = ctx.id().clone();

        // Generate child id for chat
        let chat_id = ActorId::of::<Chat>(format!("{}/chat", self_id.name()));
        self.chat_id = Some(chat_id.clone());

        // Spawn the chat actor
        let (mut chat_ctx, mut chat_actor) = Actor::spawn(
            ctx.engine().clone(),
            chat_id.clone(),
            self.chat.clone(),
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;

        // Start the chat actor
        let chat_handle = tokio::spawn(async move {
            if let Err(e) = chat_actor.start(&mut chat_ctx).await {
                error!("Chat actor error: {}", e);
            }
        });

        self.chat_handle = Some(chat_handle);
        debug!("Summary actor ready");

        Ok(())
    }
}
