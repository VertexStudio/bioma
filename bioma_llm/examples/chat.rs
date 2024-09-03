use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use ollama_rs::generation::chat::{ChatMessage, ChatMessageResponse};
use serde::{Deserialize, Serialize};
use std::future::Future;
use tracing::{error, info};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MainActor {
    llm_id: ActorId,
    max_exchanges: usize,
}

impl Actor for MainActor {
    type Error = SystemActorError;

    fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), Self::Error>> {
        async move {
            info!("{} Starting conversation with LLM", ctx.id());
            let llm_id = self.llm_id.clone();
            let mut exchanges = 0;
            let conversation_starters = vec![
                "Hello! Can you tell me about the Rust programming language?",
                "What are some key features of Rust?",
                "How does Rust ensure memory safety?",
            ];

            for starter in conversation_starters {
                if exchanges >= self.max_exchanges {
                    break;
                }

                info!("{} Sending message: {}", ctx.id(), starter);
                let chat_message = ChatMessage::user(starter.to_string());
                let response: ChatMessageResponse = ctx.send::<LLM, ChatMessage>(chat_message, &llm_id).await?;

                if let Some(assistant_message) = response.message {
                    info!("{} Received response: {}", ctx.id(), assistant_message.content);
                    exchanges += 1;
                } else {
                    error!("{} No response received from LLM", ctx.id());
                }
            }

            // Send exit message to LLM
            info!("{} Sending exit message to LLM", ctx.id());
            ctx.do_send::<LLM, llm::SystemMessage>(llm::SystemMessage::Exit, &llm_id).await?;

            info!("{} Conversation ended", ctx.id());
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Initialize the actor system
    let engine = Engine::test().await?;

    // Create actor IDs
    let llm_id = ActorId::of::<LLM>("/llm");
    let main_id = ActorId::of::<MainActor>("/main");

    // Spawn the LLM actor
    let mut llm_actor = Actor::spawn(
        &engine,
        &llm_id,
        LLM {
            model_name: "gemma2:2b".to_string(),
            generation_options: Default::default(),
            messages_number_limit: 10,
            history: Vec::new(),
            ollama: None,
        },
    )
    .await?;

    // Spawn the main actor
    let mut main_actor =
        Actor::spawn(&engine, &main_id, MainActor { llm_id: llm_id.clone(), max_exchanges: 3 }).await?;

    // Start the LLM actor
    let llm_handle = tokio::spawn(async move {
        if let Err(e) = llm_actor.start().await {
            error!("LLM actor error: {}", e);
        }
    });

    // Start the main actor
    let main_handle = tokio::spawn(async move {
        if let Err(e) = main_actor.start().await {
            error!("Main actor error: {}", e);
        }
    });

    // Wait for both actors to finish
    let _ = tokio::try_join!(llm_handle, main_handle);

    // Export the database for debugging
    dbg_export_db!(engine);

    Ok(())
}
