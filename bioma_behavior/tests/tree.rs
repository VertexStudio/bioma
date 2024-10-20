use actions::log::LogLevel::Info;
use bioma_actor::prelude::*;
use bioma_behavior::prelude::*;
use bioma_behavior::tree::Node;
// use serde::{Deserialize, Serialize};
// use std::fmt::Debug;
// use std::io::Write;
use std::time::Duration;
use test_log::test;
// use tracing::info;
// use tracing_subscriber::{fmt::MakeWriter, layer::SubscriberExt, Layer};

#[test(tokio::test)]
async fn test_tree_roundtrip() -> Result<(), SystemActorError> {
    // Initialize the engine
    let engine = Engine::test().await?;
    let output_dir = engine.output_dir().join("debug").join("test_tree_roundtrip");
    std::fs::create_dir_all(&output_dir).unwrap();

    let wait_0 = actions::Wait::builder().duration(Duration::from_secs(1)).build();
    let log_0 = actions::Log::builder().level(Info).text("Log 0".to_string()).build();
    let log_1 = actions::Log::builder().level(Info).text("Log 1".to_string()).build();
    let log_2 = actions::Log::builder().level(Info).text("Log 2".to_string()).build();
    let delay_0 = decorators::Delay::builder().duration(Duration::from_secs(1)).build();

    let wait_0 = Node::action("Wait", "wait", &wait_0);
    let log_0 = Node::action("Log", "log_0", &log_0);
    let log_1 = Node::action("Log", "log_1", &log_1);
    let log_2 = Node::action("Log", "log_2", &log_2);
    let delay_0 = Node::decorator("Delay", "delay", &delay_0, log_2);

    let all_0 = composites::Sequence::builder().build();
    let all_0 = Node::composite("Sequence", "all", &all_0, vec![wait_0, log_0, log_1, delay_0]);

    let tree = BehaviorTree { root: Some(all_0), handles: Default::default() };

    let tree_json = serde_json::to_string_pretty(&tree).unwrap();
    let tree_file = output_dir.join("tree.json");
    std::fs::write(tree_file, &tree_json).unwrap();

    let tree_deserialized: BehaviorTree = serde_json::from_str(&tree_json).unwrap();
    let tree_file_deserialized = output_dir.join("tree_deserialized.json");
    std::fs::write(tree_file_deserialized, serde_json::to_string_pretty(&tree_deserialized).unwrap()).unwrap();

    Ok(())
}
