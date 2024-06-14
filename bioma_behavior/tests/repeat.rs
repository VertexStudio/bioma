use bioma_behavior::actions::{Mock, MockMode};
use bioma_behavior::decorators::{Repeat, RepeatMode};
use bioma_behavior::prelude::*;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_behavior_repeat_log_3_times() {
    let (telemetry_tx, mut telemetry_rx) = mpsc::channel::<BehaviorTelemetry>(1000);

    let repeat_0 = BehaviorId::new("repeat-0");
    let log_0 = BehaviorId::new("mock-0");

    let bt_id = BehaviorTreeId::new("bt-0");

    let mut bt = BehaviorTreeHandle::new(BehaviorTree::new(
        &bt_id,
        &repeat_0,
        DefaultBehaviorTreeConfig::mock(),
        Some(telemetry_tx),
        None,
    ));
    bt.add_node(&repeat_0, Repeat::new(RepeatMode::Repeat(3), &log_0))
        .await;
    bt.add_node(&log_0, Mock::new("hello".to_string(), MockMode::Succeed))
        .await;

    println!("PRE-RUN: {:?}", bt);

    let status = bt.run().await;

    println!("POST-RUN: {:?}", bt);

    assert_eq!(status, Ok(BehaviorStatus::Success));

    let status = bt.shutdown().await;
    assert_eq!(status, Ok(BehaviorStatus::Shutdown));

    let mut telemetry = vec![];
    telemetry_rx.recv_many(&mut telemetry, 1000).await;

    for t in &telemetry {
        println!("{}", t);
    }

    let expected_telemetry = r#"
        [bt-0] bioma::core::Repeat(repeat-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Repeat(repeat-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Repeat(repeat-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - hello (ticks: 1)
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - hello (ticks: 2)
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - hello (ticks: 3)
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - hello (ticks: 4)
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Repeat(repeat-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Repeat(repeat-0): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - ShutdownEnd
        [bt-0] bioma::core::Repeat(repeat-0): Ok(Shutdown) - ShutdownEnd
    "#
    .trim();
    let expected_telemetry: Vec<&str> = expected_telemetry.lines().map(str::trim).collect();

    for (t, e) in telemetry.iter().zip(expected_telemetry.iter()) {
        assert_eq!(t.to_string(), *e);
    }

    assert_eq!(telemetry.len(), expected_telemetry.len());
}

#[tokio::test]
async fn test_behavior_repeat_until_failure() {
    let (telemetry_tx, mut telemetry_rx) = mpsc::channel::<BehaviorTelemetry>(1000);

    let repeat_0 = BehaviorId::new("repeat-0");
    let log_0 = BehaviorId::new("mock-0");

    let bt_id = BehaviorTreeId::new("bt-0");

    let mut bt = BehaviorTreeHandle::new(BehaviorTree::new(
        &bt_id,
        &repeat_0,
        DefaultBehaviorTreeConfig::mock(),
        Some(telemetry_tx),
        None,
    ));
    bt.add_node(
        &repeat_0,
        Repeat::new(RepeatMode::RepeatUntilFailure, &log_0),
    )
    .await;
    bt.add_node(
        &log_0,
        Mock::new("hello".to_string(), MockMode::FailOnTick(3)),
    )
    .await;

    println!("PRE-RUN: {:?}", bt);

    let status = bt.run().await;

    println!("POST-RUN: {:?}", bt);

    assert_eq!(status, Ok(BehaviorStatus::Success));

    let status = bt.shutdown().await;
    assert_eq!(status, Ok(BehaviorStatus::Shutdown));

    let mut telemetry = vec![];
    telemetry_rx.recv_many(&mut telemetry, 1000).await;

    for t in &telemetry {
        println!("{}", t);
    }

    let expected_telemetry = r#"
        [bt-0] bioma::core::Repeat(repeat-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Repeat(repeat-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Repeat(repeat-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - hello (ticks: 1)
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - hello (ticks: 2)
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - hello (ticks: 3)
        [bt-0] bioma::core::Mock(mock-0): Ok(Failure) - TickEnd
        [bt-0] bioma::core::Repeat(repeat-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Repeat(repeat-0): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Failure) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - ShutdownEnd
        [bt-0] bioma::core::Repeat(repeat-0): Ok(Shutdown) - ShutdownEnd
    "#
    .trim();
    let expected_telemetry: Vec<&str> = expected_telemetry.lines().map(str::trim).collect();

    for (t, e) in telemetry.iter().zip(expected_telemetry.iter()) {
        assert_eq!(t.to_string(), *e);
    }

    assert_eq!(telemetry.len(), expected_telemetry.len());
}

#[tokio::test]
async fn test_behavior_repeat_until_success() {
    let (telemetry_tx, mut telemetry_rx) = mpsc::channel::<BehaviorTelemetry>(1000);

    let repeat_0 = BehaviorId::new("repeat-0");
    let log_0 = BehaviorId::new("mock-0");

    let bt_id = BehaviorTreeId::new("bt-0");

    let mut bt = BehaviorTreeHandle::new(BehaviorTree::new(
        &bt_id,
        &repeat_0,
        DefaultBehaviorTreeConfig::mock(),
        Some(telemetry_tx),
        None,
    ));
    bt.add_node(
        &repeat_0,
        Repeat::new(RepeatMode::RepeatUntilSuccess, &log_0),
    )
    .await;
    bt.add_node(
        &log_0,
        Mock::new("hello".to_string(), MockMode::SucceedOnTick(3)),
    )
    .await;

    println!("PRE-RUN: {:?}", bt);

    let status = bt.run().await;

    println!("POST-RUN: {:?}", bt);

    assert_eq!(status, Ok(BehaviorStatus::Success));

    let status = bt.shutdown().await;
    assert_eq!(status, Ok(BehaviorStatus::Shutdown));

    let mut telemetry = vec![];
    telemetry_rx.recv_many(&mut telemetry, 1000).await;

    for t in &telemetry {
        println!("{}", t);
    }

    let expected_telemetry = r#"
        [bt-0] bioma::core::Repeat(repeat-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Repeat(repeat-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Repeat(repeat-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - hello (ticks: 1)
        [bt-0] bioma::core::Mock(mock-0): Ok(Failure) - TickEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Failure) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - hello (ticks: 2)
        [bt-0] bioma::core::Mock(mock-0): Ok(Failure) - TickEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Failure) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - hello (ticks: 3)
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Repeat(repeat-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Repeat(repeat-0): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - ShutdownEnd
        [bt-0] bioma::core::Repeat(repeat-0): Ok(Shutdown) - ShutdownEnd
    "#
    .trim();
    let expected_telemetry: Vec<&str> = expected_telemetry.lines().map(str::trim).collect();

    for (t, e) in telemetry.iter().zip(expected_telemetry.iter()) {
        assert_eq!(t.to_string(), *e);
    }

    assert_eq!(telemetry.len(), expected_telemetry.len());
}
