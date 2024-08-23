use crate::behavior::{Run, Status};
use crate::prelude::*;
use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::time::Duration;

/// Waits for a specified duration, then succeeds.
///
/// The `Wait` action pauses for the given duration when ticked and always returns success after the
/// delay period has elapsed.
///
#[derive(Debug, Serialize, Deserialize)]
pub struct Wait {
    #[serde(with = "humantime_serde")]
    pub duration: Duration,

    #[serde(skip)]
    node: BehaviorNode,
}

impl Wait {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            node: BehaviorNode::default(),
        }
    }
}

impl Behavior for Wait {
    const TYPE: BehaviorType = BehaviorType::Action;
    impl_behavior_node!();
}

impl MessageRx<Run> for Wait {
    fn recv(&self, _message: Run) -> impl Future<Output = Status> {
        async move {
            tokio::time::sleep(self.duration).await;
            Status::Success
        }
    }
}
