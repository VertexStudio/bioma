use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json;
use tracing::{error, info};

#[derive(Deserialize, Serialize, JsonSchema)]
struct RustPrinciples {
    /// Core principles that define Rust's design philosophy
    core_principles: Vec<String>,

    /// Memory safety features and guarantees
    memory_safety_features: Vec<String>,

    /// Concurrency model and thread safety principles
    concurrency_principles: Vec<String>,

    /// Key ownership system concepts
    ownership_concepts: Vec<String>,

    /// Additional distinguishing features
    additional_features: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MainActor;

impl Actor for MainActor {
    type Error = ChatError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        let ask_id = ActorId::of::<Chat>("/chat");
        let ask = Chat::builder().model("llama3.2".into()).build();

        // Spawn the chat actor
        info!("{} Spawning chat actor", ctx.id());
        let (mut ask_ctx, mut ask_actor) =
            Actor::spawn(ctx.engine().clone(), ask_id.clone(), ask, SpawnOptions::default()).await?;
        tokio::spawn(async move {
            if let Err(e) = ask_actor.start(&mut ask_ctx).await {
                error!("Chat actor error: {}", e);
            }
        });

        let question = "What are the key principles of Rust programming language?";
        info!("{} Asking: {}", ctx.id(), question);

        let chat_message = ChatMessage::user(question.to_string());
        let response: ChatMessageResponse = ctx
            .send_and_wait_reply::<Chat, ChatMessages>(
                ChatMessages {
                    messages: vec![chat_message],
                    restart: false,
                    persist: false,
                    stream: false,
                    format: Some(chat::Schema::new::<RustPrinciples>()),
                },
                &ask_id,
                SendOptions::builder().timeout(std::time::Duration::from_secs(100)).build(),
            )
            .await?;

        if let Some(assistant_message) = response.message {
            // Attempt to parse the response as JSON, pretty print if successful,
            // otherwise print the raw response string.
            match serde_json::from_str::<serde_json::Value>(&assistant_message.content) {
                Ok(json_value) => {
                    info!(
                        "{} Structured response:\n{}",
                        ctx.id(),
                        serde_json::to_string_pretty(&json_value).unwrap_or_else(|_| assistant_message.content.clone())
                    );
                }
                Err(_) => {
                    info!("{} Unstructured response: {}", ctx.id(), assistant_message.content);
                }
            }
        } else {
            error!("{} No response received from LLM", ctx.id());
        }

        info!("{} Request completed", ctx.id());
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
        Actor::spawn(engine.clone(), main_id.clone(), MainActor {}, SpawnOptions::default()).await?;

    // Start the main actor
    main_actor.start(&mut main_ctx).await?;

    // Export the database for debugging
    dbg_export_db!(engine);

    Ok(())
}
