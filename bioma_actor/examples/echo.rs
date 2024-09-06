/// This example demonstrates the use of the Bioma actor library.
/// It creates an actor that echoes messages back to the sender.
/// The actor will stop after a specified number of echoes.
///
/// To run this example, use the following command:
/// cargo run --example echo
///
/// Launch a surrealdb database to connect the engine to
/// surreal start --log debug --user root --password root
///
/// cargo run --example echo

use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use tracing::{error, info};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Echo {
    max_echos: usize,
}

impl Message<String> for Echo {
    type Response = String;

    fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        msg: &String,
    ) -> impl Future<Output = Result<String, Self::Error>> {
        async move {
            info!("{} Received message: {}", ctx.id(), msg);
            Ok(msg.clone())
        }
    }
}

impl Actor for Echo {
    type Error = SystemActorError;

    fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), Self::Error>> {
        async move {
            info!("{} Started", ctx.id());

            let mut stream = ctx.recv().await?;
            while let Some(Ok(frame)) = stream.next().await {
                if let Some(echo) = frame.is::<String>() {
                    let response = self.reply(ctx, &echo, &frame).await;
                    if let Err(err) = response {
                        error!("{} {:?}", ctx.id(), err);
                    }
                    if self.max_echos > 0 {
                        self.max_echos -= 1;
                    } else {
                        break;
                    }
                }
            }
            info!("{} Finished", ctx.id());
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
    let echo_id = ActorId::of::<Echo>("/echo");

    // Spawn the echo actor
    let mut echo_actor = Actor::spawn(&engine, &echo_id, Echo { max_echos: 3 }).await?;

    // Start the echo actor
    echo_actor.start().await?;

    // Export the database for debugging
    dbg_export_db!(engine);

    Ok(())
}