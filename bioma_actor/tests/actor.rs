use bioma_actor::prelude::*;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use test_log::test;
use tokio::time::{sleep, Duration};
use tracing::{error, info};

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
#[derive(Debug, Serialize, Deserialize)]
struct TestActor {
    count: usize,
}

impl Message<TestMessage> for TestActor {
    type Response = TestResponse;

    async fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        msg: &TestMessage,
    ) -> Result<Self::Response, TestActorError> {
        self.count += 1;
        let response = TestResponse { content: format!("Received: {}", msg.content), count: self.count };
        Ok(response)
    }
}

impl Actor for TestActor {
    type Error = TestActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), TestActorError> {
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(msg) = frame.is::<TestMessage>() {
                self.reply(ctx, &msg, &frame).await?;
            }
        }
        Ok(())
    }
}

// Additional actor and message types for error handling test
#[derive(Debug, Serialize, Deserialize)]
struct ErrorActor;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TriggerError;

impl Message<TriggerError> for ErrorActor {
    type Response = ();

    async fn handle(&mut self, _ctx: &mut ActorContext<Self>, _: &TriggerError) -> Result<(), TestActorError> {
        Err(TestActorError::Custom("Simulated error".to_string()))
    }
}

impl Actor for ErrorActor {
    type Error = TestActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), TestActorError> {
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(trigger) = frame.is::<TriggerError>() {
                self.reply(ctx, &trigger, &frame).await?;
            }
        }
        Ok(())
    }
}

#[test(tokio::test)]
async fn test_actor_message_handling() -> Result<(), TestActorError> {
    let engine = Engine::test().await?;

    let test_actor_id = ActorId::of::<TestActor>("/test");
    let mut test_actor = Actor::spawn(&engine, &test_actor_id, TestActor { count: 0 }, SpawnOptions::default()).await?;

    let test_handle = tokio::spawn(async move {
        if let Err(e) = test_actor.start().await {
            eprintln!("TestActor error: {}", e);
        }
    });

    let relay_actor_id = ActorId::of::<Relay>("/relay");
    let relay_actor = Actor::spawn(&engine, &relay_actor_id, Relay, SpawnOptions::default()).await?;

    // Send a message and check the response
    let message = TestMessage { content: "Hello, Actor!".to_string() };
    let response = relay_actor.send::<TestActor, TestMessage>(message, &test_actor_id, SendOptions::default()).await?;

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

    let test_actor_id = ActorId::of::<TestActor>("/test");
    let mut test_actor = Actor::spawn(&engine, &test_actor_id, TestActor { count: 0 }, SpawnOptions::default()).await?;

    let test_handle = tokio::spawn(async move {
        if let Err(e) = test_actor.start().await {
            eprintln!("TestActor error: {}", e);
        }
    });

    let relay_actor_id = ActorId::of::<Relay>("/relay");
    let relay_actor = Actor::spawn(&engine, &relay_actor_id, Relay, SpawnOptions::default()).await?;

    // Send multiple messages
    for i in 1..=5 {
        let message = TestMessage { content: format!("Message {}", i) };
        let response =
            relay_actor.send::<TestActor, TestMessage>(message, &test_actor_id, SendOptions::default()).await?;
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

    let test_actor_id = ActorId::of::<TestActor>("/test");
    let mut test_actor = Actor::spawn(&engine, &test_actor_id, TestActor { count: 0 }, SpawnOptions::default()).await?;

    let test_handle = tokio::spawn(async move {
        if let Err(e) = test_actor.start().await {
            eprintln!("TestActor error: {}", e);
        }
    });

    let relay_actor_id = ActorId::of::<Relay>("/relay");
    let relay_actor = Actor::spawn(&engine, &relay_actor_id, Relay, SpawnOptions::default()).await?;

    // Send a message to ensure the actor is working
    let message = TestMessage { content: "Lifecycle test".to_string() };
    let response = relay_actor.send::<TestActor, TestMessage>(message, &test_actor_id, SendOptions::default()).await?;
    info!("Received response: {:?}", response);
    assert_eq!(response.content, "Received: Lifecycle test");
    assert_eq!(response.count, 1);

    // Terminate the actor
    test_handle.abort();
    sleep(Duration::from_millis(100)).await;

    // Try to send a message to the terminated actor
    let message = TestMessage { content: "After termination".to_string() };
    let options = SendOptions::builder().timeout(Duration::from_secs(1)).build();
    let result = relay_actor.send::<TestActor, TestMessage>(message, &test_actor_id, options).await;
    info!("{:?}", result);
    assert!(result.is_err());

    dbg_export_db!(engine);

    Ok(())
}

#[test(tokio::test)]
async fn test_actor_error_handling() -> Result<(), TestActorError> {
    let engine = Engine::test().await?;

    let error_actor_id = ActorId::of::<ErrorActor>("/error_actor");
    let mut error_actor = Actor::spawn(&engine, &error_actor_id, ErrorActor, SpawnOptions::default()).await?;

    let error_handle = tokio::spawn(async move {
        if let Err(e) = error_actor.start().await {
            assert!(e.to_string().contains("Simulated error"));
        }
    });

    let relay_actor_id = ActorId::of::<Relay>("/relay");
    let relay_actor = Actor::spawn(&engine, &relay_actor_id, Relay, SpawnOptions::default()).await?;

    // Trigger the error
    let response =
        relay_actor.send::<ErrorActor, TriggerError>(TriggerError, &error_actor_id, SendOptions::default()).await;
    info!("{:?}", response);
    assert!(response.is_err());

    // Wait for actor to finish
    let _ = error_handle.await;

    dbg_export_db!(engine);

    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct StatefulActor {
    count: u32,
}

impl Actor for StatefulActor {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("{} Started with count: {}", ctx.id(), self.count);
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(msg) = frame.is::<IncrementCount>() {
                self.reply(ctx, &msg, &frame).await?;
            }
            self.save(ctx).await?;
        }
        Ok(())
    }
}

impl Message<IncrementCount> for StatefulActor {
    type Response = u32;

    async fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        _msg: &IncrementCount,
    ) -> Result<Self::Response, Self::Error> {
        self.count += 1;
        Ok(self.count)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IncrementCount;

#[test(tokio::test)]
async fn test_actor_state_persistence() -> Result<(), SystemActorError> {
    let engine = Engine::test().await?;

    let actor_id = ActorId::of::<StatefulActor>("/stateful_actor");

    // Spawn the actor
    let mut actor = Actor::spawn(&engine, &actor_id, StatefulActor { count: 0 }, SpawnOptions::default()).await?;

    // Start the actor
    let actor_handle = tokio::spawn(async move {
        if let Err(e) = actor.start().await {
            error!("StatefulActor error: {}", e);
        }
    });

    // Create a relay actor to send messages
    let relay_id = ActorId::of::<Relay>("/relay");
    let relay_actor = Actor::spawn(&engine, &relay_id, Relay, SpawnOptions::default()).await?;

    // Increment the count
    let response: u32 =
        relay_actor.send::<StatefulActor, IncrementCount>(IncrementCount, &actor_id, SendOptions::default()).await?;
    assert_eq!(response, 1);

    // Terminate the actor
    actor_handle.abort();
    sleep(Duration::from_millis(100)).await;

    // Respawn the actor with restore option
    let mut restored_actor = Actor::spawn(
        &engine,
        &actor_id,
        StatefulActor { count: 0 }, // This initial state should be overwritten
        SpawnOptions::builder().exists(SpawnExistsOptions::Restore).build(),
    )
    .await?;

    // Start the restored actor
    let restored_handle = tokio::spawn(async move {
        if let Err(e) = restored_actor.start().await {
            error!("Restored StatefulActor error: {}", e);
        }
    });

    // Increment the count again
    let response: u32 =
        relay_actor.send::<StatefulActor, IncrementCount>(IncrementCount, &actor_id, SendOptions::default()).await?;
    assert_eq!(response, 2);

    // Terminate the restored actor
    restored_handle.abort();

    dbg_export_db!(engine);

    Ok(())
}
