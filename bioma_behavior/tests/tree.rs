use actions::log::LogLevel::Info;
use bioma_actor::prelude::*;
use bioma_behavior::prelude::*;
use bioma_behavior::tree::Node;
use std::io::Write;
use std::time::Duration;
use test_log::test;
use tracing::debug;
use tracing_subscriber::{fmt::MakeWriter, layer::SubscriberExt, Layer};

#[test(tokio::test)]
async fn test_tree_roundtrip() -> Result<(), SystemActorError> {
    // Initialize the engine
    let engine = Engine::test().await?;
    let output_dir = engine.output_dir().join("debug").join("test_tree_roundtrip");
    std::fs::create_dir_all(&output_dir).unwrap();

    // ASCII diagram of the behavior tree:
    //
    //               Sequence (all_0)
    //            /      |      |     \
    //           /       |      |      \
    //        Wait    Log_0   Log_1   Delay
    //     (wait_0)  (log_0) (log_1) (delay_0)
    //                                  |
    //                                Log_2
    //                               (log_2)

    let wait_0 = actions::Wait::builder().duration(Duration::from_secs(1)).build();
    let log_0 = actions::Log::builder().level(Info).text("Log 0".to_string()).build();
    let log_1 = actions::Log::builder().level(Info).text("Log 1".to_string()).build();
    let log_2 = actions::Log::builder().level(Info).text("Log 2".to_string()).build();
    let delay_0 = decorators::Delay::builder().duration(Duration::from_secs(1)).build();
    let all_0 = composites::Sequence::builder().build();

    let wait_0 = Node::from("wait_0", wait_0, vec![]).unwrap();
    let log_0 = Node::from("log_0", log_0, vec![]).unwrap();
    let log_1 = Node::from("log_1", log_1, vec![]).unwrap();
    let log_2 = Node::from("log_2", log_2, vec![]).unwrap();
    let delay_0 = Node::from("delay_0", delay_0, vec![log_2]).unwrap();
    let all_0 = Node::from("all_0", all_0, vec![wait_0, log_0, log_1, delay_0]).unwrap();

    let tree = BehaviorTree { root: all_0, root_handle: None };

    let tree_json = serde_json::to_string_pretty(&tree).unwrap();
    let tree_file = output_dir.join("tree.json");
    std::fs::write(tree_file, &tree_json).unwrap();

    let tree_deserialized: BehaviorTree = serde_json::from_str(&tree_json).unwrap();
    let tree_file_deserialized = output_dir.join("tree_deserialized.json");
    std::fs::write(tree_file_deserialized, serde_json::to_string_pretty(&tree_deserialized).unwrap()).unwrap();

    // Assert that the tree is equal
    assert_eq!(tree, tree_deserialized);

    Ok(())
}

#[tokio::test]
async fn test_tree_from_file() -> Result<(), Box<dyn std::error::Error>> {
    let engine = Engine::test().await?;
    bioma_behavior::register_behaviors(&engine.registry()).await?;

    let tree_json = include_str!("../../assets/behaviors/tree.json");

    let tree: BehaviorTree = serde_json::from_str(&tree_json).unwrap();

    let tree_id = ActorId::of::<BehaviorTree>("tree_0");
    let (mut tree_ctx, mut tree_actor) =
        Actor::spawn(engine.clone(), tree_id.clone(), tree, SpawnOptions::default()).await?;

    // Create a channel for log messages
    let (log_sender, mut log_receiver) = tokio::sync::mpsc::channel::<String>(100);

    // Set up a custom layer that captures log messages
    let layer = tracing_subscriber::fmt::layer()
        .with_writer(move || TestWriter(log_sender.clone()))
        .with_filter(tracing_subscriber::filter::filter_fn(|metadata| metadata.target().starts_with("bioma_behavior")))
        .with_filter(tracing_subscriber::filter::LevelFilter::INFO);

    // Use a scoped tracing subscriber
    let _guard = tracing::subscriber::set_default(tracing_subscriber::registry().with(layer));

    tree_actor.start(&mut tree_ctx).await?;

    // Collect log messages
    let mut log_messages = Vec::new();
    while let Ok(message) = log_receiver.try_recv() {
        log_messages.push(message);
    }

    let expected_messages = vec!["Log 0", "Log 1", "Log 2"];

    for (i, expected) in expected_messages.iter().enumerate() {
        assert!(log_messages[i].contains(expected), "Log message {} does not contain expected text: {}", i, expected);
    }

    for log in &log_messages {
        debug!("{}", log);
    }

    dbg_export_db!(engine);

    Ok(())
}

struct TestWriter(tokio::sync::mpsc::Sender<String>);

impl Write for TestWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let message = String::from_utf8_lossy(buf).to_string();
        let _ = self.0.try_send(message);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for TestWriter {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

impl Clone for TestWriter {
    fn clone(&self) -> Self {
        TestWriter(self.0.clone())
    }
}
