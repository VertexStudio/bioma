use bioma_behavior::actions::{Mock, MockMode, Subtree};
use bioma_behavior::decorators::Delay;
use bioma_behavior::prelude::*;
use humantime::parse_duration;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_behavior_subtree_load() {}

#[tokio::test]
async fn test_behavior_subtree_delay_chained_2_secs() {
    let (telemetry_tx, mut telemetry_rx) = mpsc::channel::<BehaviorTelemetry>(1000);

    let duration = parse_duration("2s").unwrap();
    let now = std::time::Instant::now();

    let delay_0 = BehaviorId::new("delay-0");
    let delay_1 = BehaviorId::new("delay-1");

    let subtree_0 = BehaviorId::new("subtree-0");

    let bt_id = BehaviorTreeId::new("bt-0");

    let mut bt = BehaviorTreeHandle::new(BehaviorTree::new(
        &bt_id,
        &delay_0,
        DefaultBehaviorTreeConfig::mock(),
        Some(telemetry_tx),
        None,
    ));
    bt.add_node(&delay_0, Delay::new(duration, &delay_1)).await;

    let json_str = include_str!("../../assets/behaviors/test-subtree.json");
    bt.add_node(&delay_1, Delay::new(duration, &subtree_0))
        .await;
    bt.add_node(&subtree_0, Subtree::new(json_str.into())).await;

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
        [bt-0] bioma::core::Delay(delay-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Delay(delay-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Delay(delay-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Delay(delay-1): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Delay(delay-1): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Delay(delay-1): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Subtree(subtree-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Subtree(subtree-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Subtree(subtree-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Subtree(subtree-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Delay(delay-1): Ok(Success) - TickEnd
        [bt-0] bioma::core::Delay(delay-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Delay(delay-0): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Delay(delay-1): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Subtree(subtree-0): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Subtree(subtree-0): Ok(Shutdown) - ShutdownEnd
        [bt-0] bioma::core::Delay(delay-1): Ok(Shutdown) - ShutdownEnd
        [bt-0] bioma::core::Delay(delay-0): Ok(Shutdown) - ShutdownEnd
    "#
    .trim();
    let expected_telemetry: Vec<&str> = expected_telemetry.lines().map(str::trim).collect();

    for (t, e) in telemetry.iter().zip(expected_telemetry.iter()) {
        assert_eq!(t.to_string(), *e);
    }

    assert_eq!(telemetry.len(), expected_telemetry.len());

    let elapsed = now.elapsed() * 2;
    assert!(elapsed >= duration, "elapsed: {:?}", elapsed);
}
