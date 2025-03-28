use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use futures::StreamExt;
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

        let chat = Chat::builder().model("llama3.2".into()).build();

        // Spawn the chat actor
        let (mut chat_ctx, mut chat_actor) =
            Actor::spawn(ctx.engine().clone(), chat_id.clone(), chat, SpawnOptions::default()).await?;

        info!("{} Starting conversation with LLM", ctx.id());

        // Start the chat actor
        tokio::spawn(async move {
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

            // Get streaming response
            let mut response_stream = ctx
                .send::<Chat, ChatMessages>(
                    ChatMessages::builder().messages(vec![chat_message]).stream(true).build(),
                    &chat_id,
                    SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
                )
                .await?;

            // Process streaming response
            let mut full_response = String::new();
            while let Some(chunk) = response_stream.next().await {
                match chunk {
                    Ok(response) => {
                        print!("{}", response.message.content);
                        full_response.push_str(&response.message.content);

                        // Break if this is the final message
                        if response.done {
                            println!(); // New line after completion
                            break;
                        }
                    }
                    Err(e) => {
                        error!("{} Error receiving response: {}", ctx.id(), e);
                        break;
                    }
                }
            }

            info!("{} Full response received: {}", ctx.id(), full_response);
            exchanges += 1;
        }

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
