use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MainActor {
    max_exchanges: usize,
}

impl Actor for MainActor {
    type Error = ChatError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        let chat_id = ActorId::of::<Chat>("/llm");

        let chat = Chat::builder()
            .model_name("llama3.1".to_string())
            .generation_options(Default::default())
            .messages_number_limit(10)
            .history(Default::default())
            .build();

        // Spawn the chat actor
        let (mut chat_ctx, mut chat_actor) =
            Actor::spawn(ctx.engine().clone(), chat_id.clone(), chat, SpawnOptions::default()).await?;

        info!("{} Starting conversation with LLM", ctx.id());

        // Start the chat actor
        let chat_handle = tokio::spawn(async move {
            if let Err(e) = chat_actor.start(&mut chat_ctx).await {
                error!("LLM actor error: {}", e);
            }
        });

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
            let response: ChatMessageResponse = ctx
                .send::<Chat, ChatMessages>(
                    ChatMessages { messages: vec![chat_message], restart: false },
                    &chat_id,
                    SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
                )
                .await?;

            if let Some(assistant_message) = response.message {
                info!("{} Received response: {}", ctx.id(), assistant_message.content);
                exchanges += 1;
            } else {
                error!("{} No response received from LLM", ctx.id());
            }
        }

        chat_handle.abort();

        info!("{} Conversation ended", ctx.id());
        Ok(())
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
    let main_id = ActorId::of::<MainActor>("/main");

    // Spawn the main actor
    let (mut main_ctx, mut main_actor) =
        Actor::spawn(engine.clone(), main_id.clone(), MainActor { max_exchanges: 3 }, SpawnOptions::default()).await?;

    // Start the main actor
    main_actor.start(&mut main_ctx).await?;

    // Export the database for debugging
    dbg_export_db!(engine);

    Ok(())
}
