use bioma_actor::prelude::*;
use derive_more::Deref;
use serde::{Deserialize, Serialize};
use std::future::Future;

struct TestActor;

#[derive(Deref, Clone, Serialize, Deserialize)]
struct TestMessage(String);

impl Message for TestMessage {
    type Response = bool;
}

impl MessageRx<TestMessage> for TestActor {
    fn recv(&self, message: TestMessage) -> impl Future<Output = bool> {
        async move {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            message.len() > 5
        }
    }
}

#[tokio::test]
async fn test_message_recv_trait() {
    let actor = TestActor;
    let message = TestMessage("Hello, World!".to_string());
    let response = actor.recv(message).await;
    assert!(response);

    let message = TestMessage("Hello".to_string());
    let response = actor.recv(message).await;
    assert!(!response);
}

