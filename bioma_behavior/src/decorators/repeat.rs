use crate::prelude::*;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub enum RepeatMode {
    Repeat(usize),
    RepeatUntilSuccess,
    RepeatUntilFailure,
}

/// Repeats its child based on the specified `RepeatMode`.
///
/// The `Repeat` decorator node controls the execution of its child node according to the provided mode:
/// - `Repeat(count)`: Repeats the child node `count` times, returning the last status of the child.
/// - `RepeatUntilSuccess`: Continues to execute the child node until it succeeds, then returns `Success`.
/// - `RepeatUntilFailure`: Continues to execute the child node until it fails, then returns `Success`.
///
/// Errors from the child node are propagated immediately.
///
#[derive(Serialize, Deserialize)]
pub struct Repeat {
    pub mode: RepeatMode,
    pub child: Option<BehaviorId>,

    #[serde(skip)]
    initialized: bool,
    #[serde(skip)]
    handle: Option<BehaviorHandle>,
    #[serde(skip)]
    runtime: Option<BehaviorRuntime>,
}

impl Repeat {
    pub fn new(mode: RepeatMode, child: &BehaviorId) -> Box<Self> {
        Box::new(Self {
            mode,
            child: Some(child.clone()),
            initialized: false,
            handle: None,
            runtime: None,
        })
    }
}

#[async_trait]
#[typetag::serde(name = "bioma::core::Repeat")]
impl BehaviorNode for Repeat {
    impl_behavior_node_decorator!();

    async fn tick(&mut self) -> Result<BehaviorStatus, BehaviorError> {
        let mut count = if let RepeatMode::Repeat(count) = self.mode {
            count + 1
        } else {
            1
        };

        loop {
            // Initialize the child if not already initialized
            let child = if let Some(child) = self.handle.as_mut() {
                if self.initialized {
                    child
                } else {
                    child.init().await?;
                    self.initialized = true;
                    child
                }
            } else {
                return Err(invalid_child_error!(self));
            };

            let status = child.tick().await;
            if let RepeatMode::Repeat(_) = self.mode {
                count -= 1;
                if count == 0 {
                    return status;
                }
            } else {
                if let Ok(BehaviorStatus::Success) = status {
                    if let RepeatMode::RepeatUntilSuccess = self.mode {
                        return Ok(BehaviorStatus::Success);
                    }
                } else if let Ok(BehaviorStatus::Failure) = status {
                    if let RepeatMode::RepeatUntilFailure = self.mode {
                        return Ok(BehaviorStatus::Success);
                    }
                }
            }

            if status.is_err() {
                return status;
            }
        }
    }

    async fn init(&mut self) -> Result<BehaviorStatus, BehaviorError> {
        let tree = self.runtime()?.tree.clone();
        let Some(child) = &self.child else {
            return Err(invalid_child_error!(self));
        };
        self.handle = tree.get_node(&child).await;
        Ok(BehaviorStatus::Initialized)
    }

    async fn shutdown(&mut self) {
        // Shutdown child only if initialized
        if self.initialized {
            if let Some(child) = self.handle.as_mut() {
                child.shutdown().await;
            }
        }
        self.handle = None;
        self.initialized = false;
    }

    async fn interrupted(&mut self) {
        if let Some(child) = self.handle.as_mut() {
            child.interrupted().await;
        }
    }
}
