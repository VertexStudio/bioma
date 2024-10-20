use bioma_actor::prelude::*;
use bioma_behavior::prelude::*;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::io::Write;
// use test_log::test;
use tracing::info;
use tracing_subscriber::{fmt::MakeWriter, layer::SubscriberExt, Layer};

// MockAction behavior
#[derive(Serialize, Deserialize, Debug)]
struct MockAction {
    fact: String,
    #[serde(skip)]
    node: behavior::Action,
}

impl Behavior for MockAction {
    fn node(&self) -> behavior::Node {
        behavior::Node::Action(&self.node)
    }
}

impl Message<BehaviorTick> for MockAction {
    type Response = BehaviorStatus;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        _msg: &BehaviorTick,
    ) -> Result<BehaviorStatus, Self::Error> {
        info!("Handle {} {}", ctx.id(), self.fact);
        Ok(BehaviorStatus::Success)
    }
}

impl Actor for MockAction {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("Start {} {:?}", ctx.id(), self.node().node_type());
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(BehaviorTick) = frame.is::<BehaviorTick>() {
                self.reply(ctx, &BehaviorTick, &frame).await?;
            }
        }
        Ok(())
    }
}

// MockDecorator behavior
#[derive(Debug, Serialize, Deserialize)]
struct MockDecorator {
    #[serde(skip)]
    pub node: behavior::Decorator,
}

impl Behavior for MockDecorator {
    fn node(&self) -> behavior::Node {
        behavior::Node::Decorator(&self.node)
    }
}

impl Message<BehaviorTick> for MockDecorator {
    type Response = BehaviorStatus;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        _msg: &BehaviorTick,
    ) -> Result<BehaviorStatus, Self::Error> {
        info!("Handle {} {:?}", ctx.id(), self.node().node_type());
        let status: BehaviorStatus =
            ctx.send_as(BehaviorTick, &self.node.child.as_ref().unwrap(), SendOptions::default()).await?;
        info!("Status {} {:?}", ctx.id(), status);
        Ok(status)
    }
}

impl Actor for MockDecorator {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("Start {} {:?}", ctx.id(), self.node().node_type());
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(BehaviorTick) = frame.is::<BehaviorTick>() {
                self.reply(ctx, &BehaviorTick, &frame).await?;
            }
        }
        Ok(())
    }
}

// MockComposite behavior
#[derive(Debug, Serialize, Deserialize)]
struct MockComposite {
    #[serde(skip)]
    pub node: behavior::Composite,
}

impl Behavior for MockComposite {
    fn node(&self) -> behavior::Node {
        behavior::Node::Composite(&self.node)
    }
}

impl Message<BehaviorTick> for MockComposite {
    type Response = BehaviorStatus;

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        _msg: &BehaviorTick,
    ) -> Result<BehaviorStatus, Self::Error> {
        info!("Handle {} {:?}", ctx.id(), self.node().node_type());
        for child in &self.node.children {
            let status: BehaviorStatus = ctx.send_as(BehaviorTick, child, SendOptions::default()).await?;
            info!("Status {} {} {:?}", ctx.id(), child, status);
        }
        Ok(BehaviorStatus::Success)
    }
}

impl Actor for MockComposite {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("Start {} {:?}", ctx.id(), self.node().node_type());
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(BehaviorTick) = frame.is::<BehaviorTick>() {
                self.reply(ctx, &BehaviorTick, &frame).await?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MainActor {
    root: ActorId,
}

impl Actor for MainActor {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("Start {}", ctx.id());
        let status: BehaviorStatus = ctx.send_as(BehaviorTick, &self.root, SendOptions::default()).await?;
        info!("Status {} {:?}", ctx.id(), status);
        Ok(())
    }
}

#[tokio::test]
async fn test_behavior_mock() -> Result<(), SystemActorError> {
    // Initialize the engine
    let engine = Engine::test().await?;
    let output_dir = engine.output_dir();

    let mock_action_id_0 = ActorId::of::<MockAction>("mock_action_0");
    let mock_action_id_1 = ActorId::of::<MockAction>("mock_action_1");
    let mock_action_id_2 = ActorId::of::<MockAction>("mock_action_2");
    let mock_action_id_3 = ActorId::of::<MockAction>("mock_action_3");

    let mock_decorator_id = ActorId::of::<MockDecorator>("mock_decorator");

    let mock_composite_id = ActorId::of::<MockComposite>("mock_composite");

    let mock_action_0 = MockAction { fact: "0".to_string(), node: behavior::Action {} };
    let mock_action_1 = MockAction { fact: "1".to_string(), node: behavior::Action {} };
    let mock_action_2 = MockAction { fact: "2".to_string(), node: behavior::Action {} };
    let mock_action_3 = MockAction { fact: "3".to_string(), node: behavior::Action {} };

    let mock_composite = MockComposite {
        node: behavior::Composite {
            children: vec![
                mock_action_id_0.clone(),
                mock_action_id_1.clone(),
                mock_action_id_2.clone(),
                mock_action_id_3.clone(),
            ],
        },
    };

    let mock_decorator = MockDecorator { node: behavior::Decorator { child: Some(mock_composite_id.clone()) } };

    // Save behavior tree to output debug
    let tree = serde_json::to_string_pretty(&mock_decorator).unwrap();
    tokio::fs::write(output_dir.join("debug").join("test_behavior_mock.json"), &tree).await?;
    println!("{:#?}", &mock_decorator);

    // Spawn the actors
    let (mut mock_action_0_ctx, mut mock_action_0) =
        Actor::spawn(engine.clone(), mock_action_id_0.clone(), mock_action_0, SpawnOptions::default()).await?;
    let (mut mock_action_1_ctx, mut mock_action_1) =
        Actor::spawn(engine.clone(), mock_action_id_1.clone(), mock_action_1, SpawnOptions::default()).await?;
    let (mut mock_action_2_ctx, mut mock_action_2) =
        Actor::spawn(engine.clone(), mock_action_id_2.clone(), mock_action_2, SpawnOptions::default()).await?;
    let (mut mock_action_3_ctx, mut mock_action_3) =
        Actor::spawn(engine.clone(), mock_action_id_3.clone(), mock_action_3, SpawnOptions::default()).await?;
    let (mut mock_composite_ctx, mut mock_composite) =
        Actor::spawn(engine.clone(), mock_composite_id.clone(), mock_composite, SpawnOptions::default()).await?;
    let (mut mock_decorator_ctx, mut mock_decorator) =
        Actor::spawn(engine.clone(), mock_decorator_id.clone(), mock_decorator, SpawnOptions::default()).await?;

    // Start the actors
    tokio::spawn(async move { mock_action_0.start(&mut mock_action_0_ctx).await.unwrap() });
    tokio::spawn(async move { mock_action_1.start(&mut mock_action_1_ctx).await.unwrap() });
    tokio::spawn(async move { mock_action_2.start(&mut mock_action_2_ctx).await.unwrap() });
    tokio::spawn(async move { mock_action_3.start(&mut mock_action_3_ctx).await.unwrap() });
    tokio::spawn(async move { mock_composite.start(&mut mock_composite_ctx).await.unwrap() });
    tokio::spawn(async move { mock_decorator.start(&mut mock_decorator_ctx).await.unwrap() });

    tokio::time::sleep(std::time::Duration::from_secs(0)).await;

    // Create a channel for log messages
    let (log_sender, mut log_receiver) = tokio::sync::mpsc::channel::<String>(100);

    // Set up a custom layer that captures log messages
    let layer = tracing_subscriber::fmt::layer()
        .with_writer(move || TestWriter(log_sender.clone()))
        .with_filter(tracing_subscriber::filter::LevelFilter::INFO);

    // Use a scoped tracing subscriber
    let _guard = tracing::subscriber::set_default(tracing_subscriber::registry().with(layer));

    // Main actor
    let main_actor_id = ActorId::of::<MainActor>("main");
    let main_actor = MainActor { root: mock_decorator_id };
    let (mut main_ctx, mut main_actor) =
        Actor::spawn(engine.clone(), main_actor_id.clone(), main_actor, SpawnOptions::default()).await?;

    // Start the main actor
    main_actor.start(&mut main_ctx).await?;

    // Small delay to ensure all log messages are processed
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Collect log messages
    let mut log_messages = Vec::new();
    while let Ok(message) = log_receiver.try_recv() {
        log_messages.push(message);
    }

    // Print log messages
    // for log in &log_messages {
    //     println!("{}", log);
    // }

    assert_eq!(log_messages.len(), 13, "Expected 13 log messages");

    assert!(log_messages[0].contains("Start main:behavior::MainActor"));
    assert!(log_messages[1].contains("Handle mock_decorator:behavior::MockDecorator Decorator"));
    assert!(log_messages[2].contains("Handle mock_composite:behavior::MockComposite Composite"));
    assert!(log_messages[3].contains("Handle mock_action_0:behavior::MockAction 0"));
    assert!(log_messages[4]
        .contains("Status mock_composite:behavior::MockComposite mock_action_0:behavior::MockAction Success"));
    assert!(log_messages[5].contains("Handle mock_action_1:behavior::MockAction 1"));
    assert!(log_messages[6]
        .contains("Status mock_composite:behavior::MockComposite mock_action_1:behavior::MockAction Success"));
    assert!(log_messages[7].contains("Handle mock_action_2:behavior::MockAction 2"));
    assert!(log_messages[8]
        .contains("Status mock_composite:behavior::MockComposite mock_action_2:behavior::MockAction Success"));
    assert!(log_messages[9].contains("Handle mock_action_3:behavior::MockAction 3"));
    assert!(log_messages[10]
        .contains("Status mock_composite:behavior::MockComposite mock_action_3:behavior::MockAction Success"));
    assert!(log_messages[11].contains("Status mock_decorator:behavior::MockDecorator Success"));
    assert!(log_messages[12].contains("Status main:behavior::MainActor Success"));

    // Export the database for debugging
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
