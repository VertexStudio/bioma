use bioma_actor::prelude::*;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::future::Future;
use test_log::test;
use tracing::info;

// Custom error type for test actors
#[derive(Debug, thiserror::Error)]
enum TestActorError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Custom error: {0}")]
    Custom(String),
}

impl ActorError for TestActorError {}

// Additional actor and message types for error handling test
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ErrorActor;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TriggerError;

impl Message<TriggerError> for ErrorActor {
    type Response = ();

    fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        _: &TriggerError,
    ) -> impl Future<Output = Result<(), TestActorError>> {
        async move { Err(TestActorError::Custom("Simulated error".to_string())) }
    }
}

impl Actor for ErrorActor {
    type Error = TestActorError;

    fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), TestActorError>> {
        async move {
            let mut stream = ctx.recv().await?;
            while let Some(Ok(frame)) = stream.next().await {
                if let Some(trigger) = frame.is::<TriggerError>() {
                    self.reply(ctx, &trigger, &frame).await?;
                }
            }
            Ok(())
        }
    }
}

#[test(tokio::test)]
async fn test_actor_error_handling() -> Result<(), TestActorError> {
    let engine = Engine::test().await?;

    let error_actor_id = ActorId::of::<ErrorActor>("/error_actor");
    let mut error_actor = Actor::spawn(&engine, &error_actor_id, ErrorActor).await?;

    let error_handle = tokio::spawn(async move {
        if let Err(e) = error_actor.start().await {
            assert!(e.to_string().contains("Simulated error"));
        }
    });

    // tokio::time::sleep(Duration::from_secs(1)).await;

    let bridge_actor_id = ActorId::of::<BridgeActor>("/bridge_actor");
    let bridge_actor = Actor::spawn(&engine, &bridge_actor_id, BridgeActor).await?;

    // Trigger the error
    let response = bridge_actor.send::<ErrorActor, TriggerError>(TriggerError, &error_actor_id).await;
    info!("{:?}", response);
    assert!(response.is_err());

    // Wait for actor to finish
    let _ = error_handle.await;

    dbg_export_db!(engine);

    Ok(())
}
