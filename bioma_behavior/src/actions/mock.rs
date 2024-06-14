use crate::prelude::*;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub enum MockMode {
    Succeed,
    Fail,
    SucceedOnTick(usize),
    FailOnTick(usize),
}

/// Simulates an action for testing purposes, sending a specified message to the telemetry log.
///
/// The `Mock` action node can be configured to either always succeed or fail, or to succeed or fail
/// based on a specified number of ticks. This node helps in creating predictable behavior for
/// testing complex behavior trees without the need for actual execution of the actions.
///
/// # Modes
/// - `Succeed`: Always returns success.
/// - `Fail`: Always returns failure.
/// - `SucceedOnTick(x)`: Returns success on the x-th tick; otherwise, returns failure.
/// - `FailOnTick(x)`: Returns failure on the x-th tick; otherwise, returns success.
///
#[derive(Serialize, Deserialize)]
pub struct Mock {
    pub msg: String,
    pub mode: MockMode,

    pub inputs: IndexSet<BlackboardChannel>,
    pub outputs: IndexSet<BlackboardChannel>,

    #[serde(skip)]
    pub ticks: usize,
    #[serde(skip)]
    runtime: Option<BehaviorRuntime>,
}

impl Mock {
    pub fn new(msg: String, mode: MockMode) -> Box<Self> {
        Box::new(Self {
            msg,
            mode,
            inputs: IndexSet::new(),
            outputs: IndexSet::new(),
            ticks: 0,
            runtime: None,
        })
    }
}

#[async_trait]
#[typetag::serde(name = "bioma::core::Mock")]
impl BehaviorNode for Mock {
    impl_behavior_node_action!();
    impl_behavior_node_inputs!();
    impl_behavior_node_outputs!();

    async fn tick(&mut self) -> Result<BehaviorStatus, BehaviorError> {
        let status = self.runtime()?.status.clone();

        self.ticks += 1;
        let log = Some(format!("{} (ticks: {})", self.msg, self.ticks).into());
        behavior_telemetry(self, status.clone(), log).await;

        let blackboard = self
            .runtime
            .as_ref()
            .map(|r| r.blackboard.clone())
            .flatten();

        let telemetry_info = self.telemetry_info()?;

        let id = self.runtime()?.id.clone();

        if let Some(blackboard) = blackboard {
            for output in &self.outputs {
                let publisher = blackboard
                    .publisher::<String>(output, Some(telemetry_info.clone()), Some(status.clone()))
                    .await?;
                println!("[{}] SEND OUTPUT", id);
                let _ = publisher.send(self.msg.clone(), Some(status.clone())).await;
            }

            let mut inputs = vec![];
            for input in &self.inputs {
                let mut subscriber = blackboard
                    .subscribe::<String>(input, Some(telemetry_info.clone()), Some(status.clone()))
                    .await?;
                let status = status.clone();
                inputs.push(async move { subscriber.recv(Some(status)).await });
            }
            let pinned_inputs: Vec<std::pin::Pin<Box<_>>> =
                inputs.into_iter().map(Box::pin).collect();
            if pinned_inputs.len() > 0 {
                println!("[{}] WAITING FOR INPUTS", id);
                let _ = futures::future::select_all(pinned_inputs).await;
            }

            println!("[{}] DONE", id);
        }

        match self.mode {
            MockMode::Succeed => Ok(BehaviorStatus::Success),
            MockMode::Fail => Ok(BehaviorStatus::Failure),
            MockMode::SucceedOnTick(t) => {
                if self.ticks == t {
                    Ok(BehaviorStatus::Success)
                } else {
                    Ok(BehaviorStatus::Failure)
                }
            }
            MockMode::FailOnTick(t) => {
                if self.ticks == t {
                    Ok(BehaviorStatus::Failure)
                } else {
                    Ok(BehaviorStatus::Success)
                }
            }
        }
    }
}
