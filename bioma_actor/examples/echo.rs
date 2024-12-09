/// This example creates an actor that echoes messages back to the sender.
/// The actor will stop after a specified number of echoes.
///
/// Launch a surrealdb database to connect the engine to
///     surreal start --no-banner --allow-all --bind 0.0.0.0:9123 --user root --pass root surrealkv://output/bioma.db
///
/// To run this example, use the following command:
///     cargo run --example echo
///
/// To send a message from js, in a separate terminal:
///     cd bioma_js
///     npm install
///     node bioma.test.js
///
use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EchoText {
    text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EchoedText {
    text: String,
    echoes_left: usize,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Echo {
    max_echoes: usize,
}

impl Message<EchoText> for Echo {
    type Response = EchoedText;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, msg: &EchoText) -> Result<(), Self::Error> {
        info!("{} Received message: {:?}", ctx.id(), msg);
        self.max_echoes = self.max_echoes.saturating_sub(1);
        let echoed_text = EchoedText { text: msg.text.clone(), echoes_left: self.max_echoes };
        ctx.reply(echoed_text).await?;
        Ok(())
    }
}

impl Actor for Echo {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("{} Started", ctx.id());
        info!("{} Waiting for messages of type {}", ctx.id(), std::any::type_name::<EchoText>());

        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(echo) = frame.is::<EchoText>() {
                let response = self.reply(ctx, &echo, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
                if self.max_echoes == 0 {
                    break;
                }
            }
        }
        info!("{} Finished", ctx.id());
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
    let engine_options = EngineOptions::builder().endpoint("ws://localhost:9123".into()).build();
    let engine = Engine::connect(engine_options).await?;

    // Create echo ID
    let echo_id = ActorId::of::<Echo>("/echo");

    // Wait for the echo actor to finish
    let (mut echo_ctx, mut echo_actor) = Actor::spawn(
        engine.clone(),
        echo_id.clone(),
        Echo { max_echoes: 3 },
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await?;
    echo_actor.start(&mut echo_ctx).await?;

    Ok(())
}
