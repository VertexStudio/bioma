use crate::prelude::*;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Waits for a specified duration, then succeeds.
///
/// The `Wait` action node pauses for the given duration when ticked and always returns success after the
/// delay period has elapsed.
///
#[derive(Serialize, Deserialize)]
pub struct Wait {
    #[serde(with = "humantime_serde")]
    pub duration: Duration,

    #[serde(skip)]
    runtime: Option<BehaviorRuntime>,
}

impl Wait {
    pub fn new(duration: Duration) -> Box<Self> {
        Box::new(Self {
            duration,
            runtime: None,
        })
    }
}

#[async_trait]
#[typetag::serde(name = "bioma::core::Wait")]
impl BehaviorNode for Wait {
    impl_behavior_node_action!();

    async fn tick(&mut self) -> Result<BehaviorStatus, BehaviorError> {
        tokio::time::sleep(self.duration).await;
        Ok(BehaviorStatus::Success)
    }
}
