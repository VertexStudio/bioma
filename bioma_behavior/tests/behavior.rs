use bioma_actor::prelude::*;
use bioma_behavior::prelude::*;
use std::io::Write;
use tracing_subscriber::{fmt::MakeWriter, layer::SubscriberExt, Layer};

#[tokio::test]
async fn test_behavior_basic() -> Result<(), Box<dyn std::error::Error>> {
    run_behavior_tree_from_json(include_str!("../../assets/behaviors/tree.json")).await?;
    Ok(())
}

async fn run_behavior_tree_from_json(tree_json: &str) -> Result<(), Box<dyn std::error::Error>> {
    let engine = Engine::test().await?;
    bioma_behavior::register_behaviors(&engine.registry()).await?;

    let tree: BehaviorTree = serde_json::from_str(&tree_json)?;
    let expected_logs = tree.logs.clone();

    let tree_id = ActorId::of::<BehaviorTree>("tree_0");
    let (mut tree_ctx, mut tree_actor) =
        Actor::spawn(engine.clone(), tree_id.clone(), tree, SpawnOptions::default()).await?;

    let (log_sender, mut log_receiver) = tokio::sync::mpsc::channel::<String>(100);

    let layer = tracing_subscriber::fmt::layer()
        .with_writer(move || TestWriter(log_sender.clone()))
        .with_filter(tracing_subscriber::filter::filter_fn(|metadata| metadata.target().starts_with("bioma_behavior")))
        .with_filter(tracing_subscriber::filter::LevelFilter::INFO);

    let _guard = tracing::subscriber::set_default(tracing_subscriber::registry().with(layer));

    tree_actor.start(&mut tree_ctx).await?;

    let mut log_messages = Vec::new();
    while let Ok(message) = log_receiver.try_recv() {
        log_messages.push(message);
    }

    for (i, expected) in expected_logs.iter().enumerate() {
        assert!(log_messages[i].contains(expected), "Log message {} does not contain expected text: {}", i, expected);
    }

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
