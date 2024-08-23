use bioma_actor::{prelude::*, DB};
use futures::StreamExt;
use serde_json::json;
use test_log::test;

#[test(tokio::test)]
async fn test_actor_create_actor() -> Result<(), ActorError> {
    Engine::test().await.unwrap();
    let actor_0 = ActorId::new("actor-0").await.unwrap();
    let actor_1 = ActorId::new("actor-1").await.unwrap();
    dbg_export_db!();
    assert!(actor_0.health().await);
    assert!(actor_1.health().await);
    Ok(())
}

#[test(tokio::test)]
async fn test_actor_msg_send_recv() -> Result<(), ActorError> {
    Engine::test().await.unwrap();
    let actor_0 = ActorId::new("actor-0").await.unwrap();
    let actor_1 = ActorId::new("actor-1").await.unwrap();
    assert!(actor_0.health().await);
    assert!(actor_1.health().await);

    // Send message from actor_0 to actor_1
    let dest = actor_1.clone();
    tokio::spawn(async move {
        let msg = json!({"question": "What is your name?",});
        let response = actor_0.send("msg-test-0", &dest, &msg).await.unwrap();
        assert_eq!(
            serde_json::to_string(&response).unwrap(),
            serde_json::to_string(&json!({"answer": format!("{}", dest)})).unwrap()
        );
    });

    // Subscribe actor_1 to receiving messages
    let mut stream = actor_1.recv().await.unwrap();
    let Some(Ok(notification)) = stream.next().await else {
        panic!("Empty stream");
    };
    let request = notification.data;
    let response = json!({"answer": format!("{}", request.rx)});
    actor_1.reply(&request, response).await.unwrap();

    Ok(())
}
