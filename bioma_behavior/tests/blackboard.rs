use bioma_behavior::actions::{Mock, MockMode};
use bioma_behavior::blackboard::StringChannel;
use bioma_behavior::prelude::*;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_blackboard_core_pubsub_one() {
    let test_my_channel = BlackboardChannel::new("test/my_channel");

    let bb = BlackboardHandle::new();
    bb.add_channel(&test_my_channel, StringChannel::new(10))
        .await;

    let (telemetry_tx, mut telemetry_rx) = mpsc::channel::<BehaviorTelemetry>(1000);

    let telemetry_info = BehaviorTelemetryInfo {
        tree_id: Some(BehaviorTreeId::new("test0")),
        telemetry_tx: Some(telemetry_tx),
        ..Default::default()
    };

    let chan_pub = bb
        .publisher::<String>(&test_my_channel, Some(telemetry_info.clone()), None)
        .await
        .unwrap();
    let mut chan_sub = bb
        .subscribe::<String>(&test_my_channel, Some(telemetry_info.clone()), None)
        .await
        .unwrap();

    println!("{:?}", chan_pub);
    println!("{:?}", chan_sub);

    chan_pub.send("Hello, world!".into(), None).await.unwrap();

    let msg = chan_sub.recv(None).await.unwrap();
    assert_eq!(msg, "Hello, world!");

    println!("Message received: {}", msg);

    let mut telemetry = vec![];

    telemetry_rx.recv_many(&mut telemetry, 1000).await;
    for t in &telemetry {
        println!("{}", t);
    }

    let expected_telemetry = r#"
        [test0]: - PUB:test/my_channel
        [test0]: - SUB:test/my_channel
        [test0]: - SEND:test/my_channel "Hello, world!"
        [test0]: - RECV:test/my_channel "Hello, world!"
    "#
    .trim();
    let expected_telemetry: Vec<&str> = expected_telemetry.lines().map(str::trim).collect();

    for (t, e) in telemetry.iter().zip(expected_telemetry.iter()) {
        assert_eq!(t.to_string(), *e);
    }

    assert_eq!(telemetry.len(), expected_telemetry.len());
}

#[tokio::test]
async fn test_blackboard_core_pubsub_many() {
    let test_my_channel = BlackboardChannel::new("test/my_channel");

    let bb = BlackboardHandle::new();
    bb.add_channel(&test_my_channel, StringChannel::new(4))
        .await;

    let (telemetry_tx, mut telemetry_rx) = mpsc::channel::<BehaviorTelemetry>(1000);

    let telemetry_info = BehaviorTelemetryInfo {
        tree_id: Some(BehaviorTreeId::new("test0")),
        telemetry_tx: Some(telemetry_tx),
        ..Default::default()
    };

    let chan_pub = bb
        .publisher::<String>(&test_my_channel, Some(telemetry_info.clone()), None)
        .await
        .unwrap();
    let mut chan_sub = bb
        .subscribe::<String>(&test_my_channel, Some(telemetry_info.clone()), None)
        .await
        .unwrap();

    println!("{:?}", chan_pub);
    println!("{:?}", chan_sub);

    for i in 0..10 {
        chan_pub
            .send(format!("Message #{}", i).into(), None)
            .await
            .unwrap();
    }

    // Should lag several messages
    for _i in 0..5 {
        let _ = chan_sub.recv(None).await;
    }

    let mut telemetry = vec![];

    telemetry_rx.recv_many(&mut telemetry, 1000).await;
    for t in &telemetry {
        println!("{}", t);
    }

    let expected_telemetry = r#"
        [test0]: - PUB:test/my_channel
        [test0]: - SUB:test/my_channel
        [test0]: - SEND:test/my_channel "Message #0"
        [test0]: - SEND:test/my_channel "Message #1"
        [test0]: - SEND:test/my_channel "Message #2"
        [test0]: - SEND:test/my_channel "Message #3"
        [test0]: - SEND:test/my_channel "Message #4"
        [test0]: - SEND:test/my_channel "Message #5"
        [test0]: - SEND:test/my_channel "Message #6"
        [test0]: - SEND:test/my_channel "Message #7"
        [test0]: - SEND:test/my_channel "Message #8"
        [test0]: - SEND:test/my_channel "Message #9"
        [test0]: - RECV:test/my_channel "Message #6"
        [test0]: - RECV:test/my_channel "Message #7"
        [test0]: - RECV:test/my_channel "Message #8"
        [test0]: - RECV:test/my_channel "Message #9"
    "#
    .trim();
    let expected_telemetry: Vec<&str> = expected_telemetry.lines().map(str::trim).collect();

    for (t, e) in telemetry.iter().zip(expected_telemetry.iter()) {
        assert_eq!(t.to_string(), *e);
    }

    assert_eq!(telemetry.len(), expected_telemetry.len());
}

#[tokio::test]
async fn test_blackboard_behavior_pubsub_recv() {
    let test_my_channel = BlackboardChannel::new("test/my_channel");

    let bb = BlackboardHandle::new();
    bb.add_channel(&test_my_channel, StringChannel::new(10))
        .await;

    let (telemetry_tx, mut telemetry_rx) = mpsc::channel::<BehaviorTelemetry>(1000);

    let log_0 = BehaviorId::new("mock-0");

    let bt_id = BehaviorTreeId::new("bt-0");

    let telemetry_info = BehaviorTelemetryInfo {
        tree_id: Some(bt_id.clone()),
        telemetry_tx: Some(telemetry_tx.clone()),
        ..Default::default()
    };

    let mut mock_0 = Mock::new("hello".to_string(), MockMode::Succeed);
    mock_0.inputs.insert(test_my_channel.clone());

    let mut bt = BehaviorTreeHandle::new(BehaviorTree::new(
        &bt_id,
        &log_0,
        DefaultBehaviorTreeConfig::mock(),
        Some(telemetry_tx),
        Some(bb.clone()),
    ));
    bt.add_node(&log_0, mock_0).await;
    println!("{:?}", bt);

    let chan_pub = bb
        .publisher::<String>(&test_my_channel, Some(telemetry_info.clone()), None)
        .await
        .unwrap();
    println!("{:?}", chan_pub);

    let bt_tasks = tokio::spawn(async move {
        let status = bt.run().await;
        assert_eq!(status, Ok(BehaviorStatus::Success));

        let status = bt.shutdown().await;
        assert_eq!(status, Ok(BehaviorStatus::Shutdown));
    });

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    chan_pub.send("Hello, world!".into(), None).await.unwrap();

    bt_tasks.await.unwrap();

    let mut telemetry = vec![];

    telemetry_rx.recv_many(&mut telemetry, 1000).await;
    for t in &telemetry {
        println!("{}", t);
    }

    let expected_telemetry = r#"
        [bt-0]: - PUB:test/my_channel
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - hello (ticks: 1)
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - SUB:test/my_channel
        [bt-0]: - SEND:test/my_channel "Hello, world!"
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - RECV:test/my_channel "Hello, world!"
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - ShutdownEnd
    "#
    .trim();
    let expected_telemetry: Vec<&str> = expected_telemetry.lines().map(str::trim).collect();

    for (t, e) in telemetry.iter().zip(expected_telemetry.iter()) {
        assert_eq!(t.to_string(), *e);
    }

    assert_eq!(telemetry.len(), expected_telemetry.len());
}

#[tokio::test]
async fn test_blackboard_behavior_pubsub_send() {
    let test_my_channel = BlackboardChannel::new("test/my_channel");

    let bb = BlackboardHandle::new();
    bb.add_channel(&test_my_channel, StringChannel::new(10))
        .await;

    let (telemetry_tx, mut telemetry_rx) = mpsc::channel::<BehaviorTelemetry>(1000);

    let log_0 = BehaviorId::new("mock-0");

    let bt_id = BehaviorTreeId::new("bt-0");

    let telemetry_info = BehaviorTelemetryInfo {
        tree_id: Some(bt_id.clone()),
        telemetry_tx: Some(telemetry_tx.clone()),
        ..Default::default()
    };

    let mut mock_0 = Mock::new("hello".to_string(), MockMode::Succeed);
    mock_0.outputs.insert(test_my_channel.clone());

    let mut bt = BehaviorTreeHandle::new(BehaviorTree::new(
        &bt_id,
        &log_0,
        DefaultBehaviorTreeConfig::mock(),
        Some(telemetry_tx),
        Some(bb.clone()),
    ));
    bt.add_node(&log_0, mock_0).await;
    println!("{:?}", bt);

    let mut chan_sub = bb
        .subscribe::<String>(&test_my_channel, Some(telemetry_info), None)
        .await
        .unwrap();
    println!("{:?}", chan_sub);

    let side_tasks = tokio::spawn(async move {
        chan_sub.recv(None).await.unwrap();
    });

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    let status = bt.run().await;
    assert_eq!(status, Ok(BehaviorStatus::Success));

    let status = bt.shutdown().await;
    assert_eq!(status, Ok(BehaviorStatus::Shutdown));

    side_tasks.await.unwrap();

    let mut telemetry = vec![];

    telemetry_rx.recv_many(&mut telemetry, 1000).await;
    for t in &telemetry {
        println!("{}", t);
    }

    let expected_telemetry = r#"
        [bt-0]: - SUB:test/my_channel
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - hello (ticks: 1)
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - PUB:test/my_channel
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - SEND:test/my_channel "hello"
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - ShutdownEnd
        [bt-0]: - RECV:test/my_channel "hello"
    "#
    .trim();
    let expected_telemetry: Vec<&str> = expected_telemetry.lines().map(str::trim).collect();

    for (t, e) in telemetry.iter().zip(expected_telemetry.iter()) {
        assert_eq!(t.to_string(), *e);
    }

    assert_eq!(telemetry.len(), expected_telemetry.len());
}
