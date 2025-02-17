use bioma_actor::prelude::*;
use ollama_rs::generation::chat::ChatMessage;
use serde::{Deserialize, Serialize};
use tracing::{debug, error};

use crate::chat::{Chat, ChatError, ChatMessages};

/// Maximum number of characters to process in a text for summarization
const MAX_TEXT_LENGTH: usize = 10_000;

/// Errors that can occur during summarization
#[derive(thiserror::Error, Debug)]
pub enum SummaryError {
    /// System-level actor errors
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    /// Chat-related errors during summarization
    #[error("Chat error: {0}")]
    Chat(#[from] ChatError),
    /// Error when chat actor is not properly initialized
    #[error("Chat actor not initialized")]
    ChatActorNotInitialized,
}

impl ActorError for SummaryError {}

/// Request to generate a summary for a given text
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummarizeText {
    /// The text content to summarize
    pub text: String,
    /// The URI/path of the source document
    pub uri: String,
}

/// Response containing the generated summary
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummaryResponse {
    /// The generated summary in markdown format, including URI and summary text
    pub summary: String,
}

/// Actor responsible for generating summaries of text content
///
/// The Summary actor uses a Chat actor to generate concise summaries of text content.
/// It handles text truncation, formatting, and communication with the underlying chat model.
///
/// # Example
/// ```rust,no_run
/// use bioma_actor::prelude::*;
/// use bioma_llm::summary::{Summary, SummarizeText};
///
/// async fn summarize_text(ctx: &ActorContext<MyActor>, text: String, uri: String) -> Result<String, Error> {
///     let summary_id = ActorId::of::<Summary>("/summary");
///     let response = ctx.send_and_wait_reply::<Summary, SummarizeText>(
///         SummarizeText { text, uri },
///         &summary_id,
///         SendOptions::default()
///     ).await?;
///     Ok(response.summary)
/// }
/// ```
#[derive(bon::Builder, Debug, Serialize, Deserialize)]
pub struct Summary {
    /// The chat actor configuration
    pub chat: Chat,
    /// ID of the spawned chat actor
    chat_id: Option<ActorId>,
    /// Handle to the chat actor's task
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

        let summary = self.generate_summary(ctx, message, chat_id).await?;
        ctx.reply(summary).await?;
        Ok(())
    }
}

impl Summary {
    /// Initialize the Summary actor by spawning and starting its chat actor
    pub async fn init(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), SummaryError> {
        // Used to namespace child actors
        let self_id = ctx.id().clone();

        // Generate child id for chat
        let chat_id = ActorId::of::<Chat>(format!("{}/chat", self_id.name()));
        self.chat_id = Some(chat_id.clone());

        // Spawn and start the chat actor
        let (mut chat_ctx, mut chat_actor) = Actor::spawn(
            ctx.engine().clone(),
            chat_id.clone(),
            self.chat.clone(),
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;

        let chat_handle = tokio::spawn(async move {
            if let Err(e) = chat_actor.start(&mut chat_ctx).await {
                error!("Chat actor error: {}", e);
            }
        });

        self.chat_handle = Some(chat_handle);
        debug!("Summary actor ready");

        Ok(())
    }

    /// Generate a summary for the given text using the chat actor
    async fn generate_summary(
        &self,
        ctx: &ActorContext<Self>,
        message: &SummarizeText,
        chat_id: &ActorId,
    ) -> Result<SummaryResponse, SummaryError> {
        // Prepare text by truncating if needed
        let truncated_text = truncate_text(&message.text, MAX_TEXT_LENGTH);
        let prompt = create_summary_prompt(&truncated_text);

        // Get summary from chat actor
        let response = ctx
            .send_and_wait_reply::<Chat, ChatMessages>(
                ChatMessages::builder().messages(vec![ChatMessage::user(prompt)]).build(),
                chat_id,
                SendOptions::builder().timeout(std::time::Duration::from_secs(300)).build(),
            )
            .await?;

        // Format response in markdown
        let summary = format_summary_markdown(&message.uri, &response.message.content);
        Ok(SummaryResponse { summary })
    }
}

/// Truncate text to the specified maximum length, adding an indicator if truncated
fn truncate_text(text: &str, max_length: usize) -> String {
    if text.len() <= max_length {
        text.to_string()
    } else {
        let mut truncated = text.chars().take(max_length).collect::<String>();
        truncated.push_str("\n... (text truncated)");
        truncated
    }
}

/// Create a prompt for the chat model to generate a summary
fn create_summary_prompt(text: &str) -> String {
    format!("Provide a concise summary of the following. Focus on the key points and main ideas:\n\n{}", text)
}

/// Format the summary in markdown with the URI
fn format_summary_markdown(uri: &str, summary: &str) -> String {
    format!("**URI**: {}\n**Summary**: {}\n", uri, summary)
}
