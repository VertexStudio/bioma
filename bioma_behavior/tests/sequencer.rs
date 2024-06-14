use bioma_behavior::actions::{Mock, MockMode};
use bioma_behavior::composites::Sequencer;
use bioma_behavior::prelude::*;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_behavior_sequencer_1st_fails() {
    let (telemetry_tx, mut telemetry_rx) = mpsc::channel::<BehaviorTelemetry>(1000);

    let sequencer_0 = BehaviorId::new("sequencer-0");

    let log_0 = BehaviorId::new("mock-0");
    let log_1 = BehaviorId::new("mock-1");

    let bt_id = BehaviorTreeId::new("bt-0");

    let mut bt = BehaviorTreeHandle::new(BehaviorTree::new(
        &bt_id,
        &sequencer_0,
        DefaultBehaviorTreeConfig::mock(),
        Some(telemetry_tx),
        None,
    ));
    let children = IndexSet::from_iter([log_0.clone(), log_1.clone()]);
    bt.add_node(&sequencer_0, Sequencer::new(children)).await;
    bt.add_node(&log_0, Mock::new("hello".to_string(), MockMode::Fail))
        .await;
    bt.add_node(&log_1, Mock::new("world".to_string(), MockMode::Succeed))
        .await;

    println!("PRE-RUN: {:?}", bt);

    let status = bt.run().await;

    println!("POST-RUN: {:?}", bt);

    assert_eq!(status, Ok(BehaviorStatus::Failure));

    let status = bt.shutdown().await;
    assert_eq!(status, Ok(BehaviorStatus::Shutdown));

    let mut telemetry = vec![];
    telemetry_rx.recv_many(&mut telemetry, 1000).await;

    for t in &telemetry {
        println!("{}", t);
    }

    let expected_telemetry = r#"
        [bt-0] bioma::core::Sequencer(sequencer-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Sequencer(sequencer-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Sequencer(sequencer-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - hello (ticks: 1)
        [bt-0] bioma::core::Mock(mock-0): Ok(Failure) - TickEnd
        [bt-0] bioma::core::Sequencer(sequencer-0): Ok(Failure) - TickEnd
        [bt-0] bioma::core::Sequencer(sequencer-0): Ok(Failure) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Failure) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - ShutdownEnd
        [bt-0] bioma::core::Sequencer(sequencer-0): Ok(Shutdown) - ShutdownEnd
    "#
    .trim();
    let expected_telemetry: Vec<&str> = expected_telemetry.lines().map(str::trim).collect();

    for (t, e) in telemetry.iter().zip(expected_telemetry.iter()) {
        assert_eq!(t.to_string(), *e);
    }

    assert_eq!(telemetry.len(), expected_telemetry.len());
}

#[tokio::test]
async fn test_behavior_sequencer_2nd_fails() {
    let (telemetry_tx, mut telemetry_rx) = mpsc::channel::<BehaviorTelemetry>(1000);

    let sequencer_0 = BehaviorId::new("sequencer-0");

    let log_0 = BehaviorId::new("mock-0");
    let log_1 = BehaviorId::new("mock-1");

    let bt_id = BehaviorTreeId::new("bt-0");

    let mut bt = BehaviorTreeHandle::new(BehaviorTree::new(
        &bt_id,
        &sequencer_0,
        DefaultBehaviorTreeConfig::mock(),
        Some(telemetry_tx),
        None,
    ));
    let children = IndexSet::from_iter([log_0.clone(), log_1.clone()]);
    bt.add_node(&sequencer_0, Sequencer::new(children)).await;
    bt.add_node(&log_0, Mock::new("hello".to_string(), MockMode::Succeed))
        .await;
    bt.add_node(&log_1, Mock::new("world".to_string(), MockMode::Fail))
        .await;

    println!("PRE-RUN: {:?}", bt);

    let status = bt.run().await;

    println!("POST-RUN: {:?}", bt);

    assert_eq!(status, Ok(BehaviorStatus::Failure));

    let status = bt.shutdown().await;
    assert_eq!(status, Ok(BehaviorStatus::Shutdown));

    let mut telemetry = vec![];
    telemetry_rx.recv_many(&mut telemetry, 1000).await;

    for t in &telemetry {
        println!("{}", t);
    }

    let expected_telemetry = r#"
        [bt-0] bioma::core::Sequencer(sequencer-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Sequencer(sequencer-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Sequencer(sequencer-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - hello (ticks: 1)
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Mock(mock-1): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Mock(mock-1): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Mock(mock-1): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-1): Ok(Running) - world (ticks: 1)
        [bt-0] bioma::core::Mock(mock-1): Ok(Failure) - TickEnd
        [bt-0] bioma::core::Sequencer(sequencer-0): Ok(Failure) - TickEnd
        [bt-0] bioma::core::Sequencer(sequencer-0): Ok(Failure) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-1): Ok(Failure) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-1): Ok(Shutdown) - ShutdownEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - ShutdownEnd
        [bt-0] bioma::core::Sequencer(sequencer-0): Ok(Shutdown) - ShutdownEnd
    "#
    .trim();
    let expected_telemetry: Vec<&str> = expected_telemetry.lines().map(str::trim).collect();

    for (t, e) in telemetry.iter().zip(expected_telemetry.iter()) {
        assert_eq!(t.to_string(), *e);
    }

    assert_eq!(telemetry.len(), expected_telemetry.len());
}

#[tokio::test]
async fn test_behavior_sequencer_all_succeed() {
    let (telemetry_tx, mut telemetry_rx) = mpsc::channel::<BehaviorTelemetry>(1000);

    let sequencer_0 = BehaviorId::new("sequencer-0");

    let log_0 = BehaviorId::new("mock-0");
    let log_1 = BehaviorId::new("mock-1");

    let bt_id = BehaviorTreeId::new("bt-0");

    let mut bt = BehaviorTreeHandle::new(BehaviorTree::new(
        &bt_id,
        &sequencer_0,
        DefaultBehaviorTreeConfig::mock(),
        Some(telemetry_tx),
        None,
    ));
    let children = IndexSet::from_iter([log_0.clone(), log_1.clone()]);
    bt.add_node(&sequencer_0, Sequencer::new(children)).await;
    bt.add_node(&log_0, Mock::new("hello".to_string(), MockMode::Succeed))
        .await;
    bt.add_node(&log_1, Mock::new("world".to_string(), MockMode::Succeed))
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
        [bt-0] bioma::core::Sequencer(sequencer-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Sequencer(sequencer-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Sequencer(sequencer-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - hello (ticks: 1)
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Mock(mock-1): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Mock(mock-1): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Mock(mock-1): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-1): Ok(Running) - world (ticks: 1)
        [bt-0] bioma::core::Mock(mock-1): Ok(Success) - TickEnd
        [bt-0] bioma::core::Sequencer(sequencer-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Sequencer(sequencer-0): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-1): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-1): Ok(Shutdown) - ShutdownEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - ShutdownEnd
        [bt-0] bioma::core::Sequencer(sequencer-0): Ok(Shutdown) - ShutdownEnd
    "#
    .trim();
    let expected_telemetry: Vec<&str> = expected_telemetry.lines().map(str::trim).collect();

    for (t, e) in telemetry.iter().zip(expected_telemetry.iter()) {
        assert_eq!(t.to_string(), *e);
    }

    assert_eq!(telemetry.len(), expected_telemetry.len());
}
