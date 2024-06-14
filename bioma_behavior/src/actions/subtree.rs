use crate::prelude::*;
use crate::serializer::json::*;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::select;
use tokio::sync::mpsc;

/// Subtree
///
/// The `Subtree` decorator node allows you to create a subtree of behavior nodes that can be executed as a single unit.
/// The Subtree will execute its child subtree recursively, until all child nodes have completed their execution.
/// The status of the child subtree is returned as the result of this node.
///
#[derive(Serialize, Deserialize)]
pub struct Subtree {
    pub source: String,

    pub inputs: IndexSet<BlackboardChannel>,
    pub outputs: IndexSet<BlackboardChannel>,

    #[serde(skip)]
    runtime: Option<BehaviorRuntime>,
    #[serde(skip)]
    subtree_bt: Option<BehaviorTreeHandle>,
    #[serde(skip)]
    subtree_teletry_rx: Option<mpsc::Receiver<BehaviorTelemetry>>,
}

impl Subtree {
    pub fn new(source: String) -> Box<Self> {
        Box::new(Self {
            source,
            inputs: IndexSet::new(),
            outputs: IndexSet::new(),
            runtime: None,
            subtree_bt: None,
            subtree_teletry_rx: None,
        })
    }

    pub fn from_path(path: PathBuf) -> Result<Box<Self>, BehaviorError> {
        let Ok(source) = std::fs::read_to_string(&path) else {
            return Err(BehaviorError::LoadError(path.clone()));
        };
        Ok(Self::new(source))
    }
}

#[async_trait]
#[typetag::serde(name = "bioma::core::Subtree")]
impl BehaviorNode for Subtree {
    impl_behavior_node_action!();
    impl_behavior_node_inputs!();
    impl_behavior_node_outputs!();

    async fn tick(&mut self) -> Result<BehaviorStatus, BehaviorError> {
        if self.subtree_bt.is_none() {
            // Teletry
            let (telemetry_tx, telemetry_rx) = mpsc::channel::<BehaviorTelemetry>(1000);
            self.subtree_teletry_rx = Some(telemetry_rx);

            let json_value = serde_json::from_str(&self.source).unwrap();
            let bt = from_json::<DefaultBehaviorTreeConfig>(&json_value, Some(telemetry_tx))
                .await
                .unwrap();
            self.subtree_bt = Some(bt);
        }

        let Some(mut subtree_telemetry_rx) = self.subtree_teletry_rx.take() else {
            return Err(BehaviorError::RuntimeNotInitialized);
        };

        let Some(mut bt) = self.subtree_bt.clone() else {
            return Err(BehaviorError::RuntimeNotInitialized);
        };

        loop {
            select! {
                msg = subtree_telemetry_rx.recv() => {
                    println!("Subtree telemetry: {:?}", msg);
                },
                status = bt.run() => {
                    return status;
                }
            }
        }
    }

    async fn init(&mut self) -> Result<BehaviorStatus, BehaviorError> {
        Ok(BehaviorStatus::Initialized)
    }

    async fn shutdown(&mut self) {
        // Shutdown subtree
    }

    async fn interrupted(&mut self) {
        // Interrupt subtree
    }
}
