use bioma_behavior::actions::*;
use bioma_behavior::blackboard::StringChannel;
use bioma_behavior::composites::*;
use bioma_behavior::decorators::*;
use bioma_behavior::prelude::*;
use bioma_behavior::serializer::json::*;
use humantime::parse_duration;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_behavior_serializer_json() {
    let (telemetry_tx, mut telemetry_rx) = mpsc::channel::<BehaviorTelemetry>(1000);

    let my_channel_0 = BlackboardChannel::new("test/my_channel/0");
    let my_channel_1 = BlackboardChannel::new("test/my_channel/1");

    let bb = BlackboardHandle::new();
    bb.add_channel(&my_channel_0, StringChannel::new(10)).await;
    bb.add_channel(&my_channel_1, StringChannel::new(11)).await;

    let duration_0 = parse_duration("2s").unwrap();
    let duration_1 = parse_duration("1s").unwrap();

    let all_0 = BehaviorId::new("all-0");

    let delay_0 = BehaviorId::new("delay-0");
    let delay_1 = BehaviorId::new("delay-1");

    let mock_0 = BehaviorId::new("mock-0");
    let mock_1 = BehaviorId::new("mock-1");

    let bt_id = BehaviorTreeId::new("bt-0");

    let mut mock_node_0 = Mock::new("from hello 0".to_string(), MockMode::Succeed);
    mock_node_0.outputs.insert(my_channel_0.clone());

    let mut mock_node_1 = Mock::new("from hello 1".to_string(), MockMode::Succeed);
    mock_node_1.inputs.insert(my_channel_0.clone());
    mock_node_1.outputs.insert(my_channel_1.clone());

    let mut bt = BehaviorTreeHandle::new(BehaviorTree::new(
        &bt_id,
        &all_0,
        DefaultBehaviorTreeConfig::mock(),
        None,
        Some(bb),
    ));
    let children = IndexSet::from_iter([delay_0.clone(), delay_1.clone()]);
    bt.add_node(&all_0, All::new(children)).await;
    bt.add_node(&delay_0, Delay::new(duration_0, &mock_0)).await;
    bt.add_node(&delay_1, Delay::new(duration_1, &mock_1)).await;
    bt.add_node(&mock_0, mock_node_0).await;
    bt.add_node(&mock_1, mock_node_1).await;

    println!("{:?}", bt);

    let json = to_json::<DefaultBehaviorTreeConfig>(&bt).await.unwrap();
    println!(
        "PRE-SERIALIZED: {}",
        serde_json::to_string_pretty(&json).unwrap()
    );

    let mut bt = from_json::<DefaultBehaviorTreeConfig>(&json, Some(telemetry_tx))
        .await
        .unwrap();

    let json = to_json::<DefaultBehaviorTreeConfig>(&bt).await.unwrap();
    println!(
        "POST-SERIALIZED: {}",
        serde_json::to_string_pretty(&json).unwrap()
    );

    println!("{:?}", bt);

    let status = bt.run().await;

    println!("{:?}", bt);

    assert_eq!(status, Ok(BehaviorStatus::Success));

    let status = bt.shutdown().await;
    assert_eq!(status, Ok(BehaviorStatus::Shutdown));

    let mut telemetry = vec![];
    telemetry_rx.recv_many(&mut telemetry, 1000).await;

    for t in &telemetry {
        println!("{}", t);
    }

    let expected_telemetry = r#"
        [bt-0] bioma::core::All(all-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Delay(delay-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Delay(delay-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Delay(delay-1): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Delay(delay-1): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::All(all-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::All(all-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Delay(delay-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Delay(delay-1): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-1): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Mock(mock-1): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Mock(mock-1): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-1): Ok(Running) - from hello 1 (ticks: 1)
        [bt-0] bioma::core::Mock(mock-1): Ok(Running) - PUB:test/my_channel/1
        [bt-0] bioma::core::Mock(mock-1): Ok(Running) - SEND:test/my_channel/1 "from hello 1"
        [bt-0] bioma::core::Mock(mock-1): Ok(Running) - SUB:test/my_channel/0
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - from hello 0 (ticks: 1)
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - PUB:test/my_channel/0
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - SEND:test/my_channel/0 "from hello 0"
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Delay(delay-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Mock(mock-1): Ok(Running) - RECV:test/my_channel/0 "from hello 0"
        [bt-0] bioma::core::Mock(mock-1): Ok(Success) - TickEnd
        [bt-0] bioma::core::Delay(delay-1): Ok(Success) - TickEnd
        [bt-0] bioma::core::All(all-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::All(all-0): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Delay(delay-1): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-1): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-1): Ok(Shutdown) - ShutdownEnd
        [bt-0] bioma::core::Delay(delay-1): Ok(Shutdown) - ShutdownEnd
        [bt-0] bioma::core::Delay(delay-0): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - ShutdownEnd
        [bt-0] bioma::core::Delay(delay-0): Ok(Shutdown) - ShutdownEnd
        [bt-0] bioma::core::All(all-0): Ok(Shutdown) - ShutdownEnd
    "#
    .trim();
    let expected_telemetry: Vec<&str> = expected_telemetry.lines().map(str::trim).collect();

    for (t, e) in telemetry.iter().zip(expected_telemetry.iter()) {
        assert_eq!(t.to_string(), *e);
    }

    assert_eq!(telemetry.len(), expected_telemetry.len());
}

#[tokio::test]
async fn test_behavior_serializer_from_json_file() {
    let (telemetry_tx, mut telemetry_rx) = mpsc::channel::<BehaviorTelemetry>(1000);

    let json_str = include_str!("../../assets/behaviors/test.json");
    let json_value = serde_json::from_str(json_str).unwrap();
    println!("{:?}", json_value);

    let mut bt = from_json::<DefaultBehaviorTreeConfig>(&json_value, Some(telemetry_tx)).await.unwrap();

    println!("{:?}", bt);

    let status = bt.run().await;

    println!("{:?}", bt);

    assert_eq!(status, Ok(BehaviorStatus::Success));

    let status = bt.shutdown().await;
    assert_eq!(status, Ok(BehaviorStatus::Shutdown));

    let mut telemetry = vec![];
    telemetry_rx.recv_many(&mut telemetry, 1000).await;

    for t in &telemetry {
        println!("{}", t);
    }

    let expected_telemetry = r#"
        [bt-0] bioma::core::All(all-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Delay(delay-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Delay(delay-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Delay(delay-1): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Delay(delay-1): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::All(all-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::All(all-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Delay(delay-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Delay(delay-1): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-1): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Mock(mock-1): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Mock(mock-1): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-1): Ok(Running) - from hello 1 (ticks: 1)
        [bt-0] bioma::core::Mock(mock-1): Ok(Running) - PUB:test/my_channel/1
        [bt-0] bioma::core::Mock(mock-1): Ok(Running) - SEND:test/my_channel/1 "from hello 1"
        [bt-0] bioma::core::Mock(mock-1): Ok(Running) - SUB:test/my_channel/0
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - InitBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - InitEnd
        [bt-0] bioma::core::Mock(mock-0): Ok(Initialized) - TickBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - from hello 0 (ticks: 1)
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - PUB:test/my_channel/0
        [bt-0] bioma::core::Mock(mock-0): Ok(Running) - SEND:test/my_channel/0 "from hello 0"
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Delay(delay-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::Mock(mock-1): Ok(Running) - RECV:test/my_channel/0 "from hello 0"
        [bt-0] bioma::core::Mock(mock-1): Ok(Success) - TickEnd
        [bt-0] bioma::core::Delay(delay-1): Ok(Success) - TickEnd
        [bt-0] bioma::core::All(all-0): Ok(Success) - TickEnd
        [bt-0] bioma::core::All(all-0): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Delay(delay-1): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-1): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-1): Ok(Shutdown) - ShutdownEnd
        [bt-0] bioma::core::Delay(delay-1): Ok(Shutdown) - ShutdownEnd
        [bt-0] bioma::core::Delay(delay-0): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Success) - ShutdownBegin
        [bt-0] bioma::core::Mock(mock-0): Ok(Shutdown) - ShutdownEnd
        [bt-0] bioma::core::Delay(delay-0): Ok(Shutdown) - ShutdownEnd
        [bt-0] bioma::core::All(all-0): Ok(Shutdown) - ShutdownEnd
    "#
    .trim();
    let expected_telemetry: Vec<&str> = expected_telemetry.lines().map(str::trim).collect();

    for (t, e) in telemetry.iter().zip(expected_telemetry.iter()) {
        assert_eq!(t.to_string(), *e);
    }

    assert_eq!(telemetry.len(), expected_telemetry.len());
}
