use bioma_actor::prelude::*;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::future::Future;
use test_log::test;
use tokio::time::{sleep, Duration};
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

// Test message types
#[derive(Clone, Debug, Serialize, Deserialize)]
struct TestMessage {
    content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TestResponse {
    content: String,
    count: usize,
}

// Test actor for basic message handling
#[derive(Clone, Debug, Serialize, Deserialize)]
struct TestActor {
    count: usize,
}

impl Message<TestMessage> for TestActor {
    type Response = TestResponse;

    fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        msg: &TestMessage,
    ) -> impl Future<Output = Result<Self::Response, TestActorError>> {
        self.count += 1;
        let response = TestResponse { content: format!("Received: {}", msg.content), count: self.count };
        async move { Ok(response) }
    }
}

impl Actor for TestActor {
    type Error = TestActorError;

    fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), TestActorError>> {
        async move {
            let mut stream = ctx.recv().await?;
            while let Some(Ok(frame)) = stream.next().await {
                if let Some(msg) = frame.is::<TestMessage>() {
                    self.reply(ctx, &msg, &frame).await?;
                }
            }
            Ok(())
        }
    }
}

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
async fn test_actor_message_handling() -> Result<(), TestActorError> {
    let engine = Engine::test().await?;

    let test_actor_id = ActorId::of::<TestActor>("/test_actor");
    let mut test_actor = Actor::spawn(&engine, &test_actor_id, TestActor { count: 0 }).await?;

    let test_handle = tokio::spawn(async move {
        if let Err(e) = test_actor.start().await {
            eprintln!("TestActor error: {}", e);
        }
    });

    let bridge_actor_id = ActorId::of::<BridgeActor>("/bridge_actor");
    let bridge_actor = Actor::spawn(&engine, &bridge_actor_id, BridgeActor).await?;

    // Send a message and check the response
    let message = TestMessage { content: "Hello, Actor!".to_string() };
    let response = bridge_actor.send::<TestActor, TestMessage>(message, &test_actor_id, SendOptions::default()).await?;

    info!("Received response: {:?}", response);
    assert_eq!(response.content, "Received: Hello, Actor!");
    assert_eq!(response.count, 1);

    // Terminate the actor
    test_handle.abort();

    dbg_export_db!(engine);

    Ok(())
}

#[test(tokio::test)]
async fn test_actor_multiple_messages() -> Result<(), TestActorError> {
    let engine = Engine::test().await?;

    let test_actor_id = ActorId::of::<TestActor>("/test_actor");
    let mut test_actor = Actor::spawn(&engine, &test_actor_id, TestActor { count: 0 }).await?;

    let test_handle = tokio::spawn(async move {
        if let Err(e) = test_actor.start().await {
            eprintln!("TestActor error: {}", e);
        }
    });

    let bridge_actor_id = ActorId::of::<BridgeActor>("/bridge_actor");
    let bridge_actor = Actor::spawn(&engine, &bridge_actor_id, BridgeActor).await?;

    // Send multiple messages
    for i in 1..=5 {
        let message = TestMessage { content: format!("Message {}", i) };
        let response =
            bridge_actor.send::<TestActor, TestMessage>(message, &test_actor_id, SendOptions::default()).await?;
        info!("Received response: {:?}", response);
        assert_eq!(response.content, format!("Received: Message {}", i));
        assert_eq!(response.count, i);
    }

    // Terminate the actor
    test_handle.abort();

    dbg_export_db!(engine);

    Ok(())
}

#[test(tokio::test)]
async fn test_actor_lifecycle() -> Result<(), TestActorError> {
    let engine = Engine::test().await?;

    let test_actor_id = ActorId::of::<TestActor>("/test_actor");
    let mut test_actor = Actor::spawn(&engine, &test_actor_id, TestActor { count: 0 }).await?;

    let test_handle = tokio::spawn(async move {
        if let Err(e) = test_actor.start().await {
            eprintln!("TestActor error: {}", e);
        }
    });

    let bridge_actor_id = ActorId::of::<BridgeActor>("/bridge_actor");
    let bridge_actor = Actor::spawn(&engine, &bridge_actor_id, BridgeActor).await?;

    // Send a message to ensure the actor is working
    let message = TestMessage { content: "Lifecycle test".to_string() };
    let response = bridge_actor.send::<TestActor, TestMessage>(message, &test_actor_id, SendOptions::default()).await?;
    info!("Received response: {:?}", response);
    assert_eq!(response.content, "Received: Lifecycle test");
    assert_eq!(response.count, 1);

    // Terminate the actor
    test_handle.abort();
    sleep(Duration::from_millis(100)).await;

    // Try to send a message to the terminated actor
    let message = TestMessage { content: "After termination".to_string() };
    let options = SendOptions::builder().timeout(Duration::from_secs(1)).build();
    let result = bridge_actor.send::<TestActor, TestMessage>(message, &test_actor_id, options).await;
    info!("{:?}", result);
    assert!(result.is_err());

    dbg_export_db!(engine);

    Ok(())
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

    let bridge_actor_id = ActorId::of::<BridgeActor>("/bridge_actor");
    let bridge_actor = Actor::spawn(&engine, &bridge_actor_id, BridgeActor).await?;

    // Trigger the error
    let response =
        bridge_actor.send::<ErrorActor, TriggerError>(TriggerError, &error_actor_id, SendOptions::default()).await;
    info!("{:?}", response);
    assert!(response.is_err());

    // Wait for actor to finish
    let _ = error_handle.await;

    dbg_export_db!(engine);

    Ok(())
}
