use crate::prelude::*;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Delays execution before proceeding with its child node.
///
/// The `Delay` decorator node pauses for a specified duration before executing its child node. It returns the result
/// of the child node's execution.
///
#[derive(Serialize, Deserialize)]
pub struct Delay {
    #[serde(with = "humantime_serde")]
    pub duration: Duration,
    pub child: Option<BehaviorId>,

    #[serde(skip)]
    initialized: bool,
    #[serde(skip)]
    handle: Option<BehaviorHandle>,
    #[serde(skip)]
    runtime: Option<BehaviorRuntime>,
}

impl Delay {
    pub fn new(duration: Duration, child: &BehaviorId) -> Box<Self> {
        Box::new(Self {
            duration,
            child: Some(child.clone()),
            initialized: false,
            handle: None,
            runtime: None,
        })
    }
}

#[async_trait]
#[typetag::serde(name = "bioma::core::Delay")]
impl BehaviorNode for Delay {
    impl_behavior_node_decorator!();

    async fn tick(&mut self) -> Result<BehaviorStatus, BehaviorError> {
        tokio::time::sleep(self.duration).await;

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

        // Tick and return child status
        child.tick().await
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
