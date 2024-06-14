use crate::prelude::*;
use async_trait::async_trait;
use indexmap::IndexSet;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Executes child nodes sequentially until one succeeds or all fail.
///
/// The `Selector` composite node processes its children one by one in order. It returns success as soon as one
/// child node succeeds. If a child fails, it proceeds to the next one. If all children fail,
/// then the `Selector` node fails.
///
#[derive(Serialize, Deserialize)]
pub struct Selector {
    pub children: IndexSet<BehaviorId>,

    #[serde(skip)]
    initialized: HashSet<BehaviorId>,
    #[serde(skip)]
    handles: HashMap<BehaviorId, BehaviorHandle>,
    #[serde(skip)]
    runtime: Option<BehaviorRuntime>,
}

impl Selector {
    pub fn new(children: IndexSet<BehaviorId>) -> Box<Self> {
        Box::new(Self {
            children,
            initialized: HashSet::new(),
            handles: HashMap::new(),
            runtime: None,
        })
    }
}

#[async_trait]
#[typetag::serde(name = "bioma::core::Selector")]
impl BehaviorNode for Selector {
    impl_behavior_node_composite!();

    async fn tick(&mut self) -> Result<BehaviorStatus, BehaviorError> {
        for child_id in &self.children {
            let child = if self.initialized.contains(child_id) {
                self.handles.get_mut(child_id)
            } else {
                None
            };

            let result = if let Some(child) = child {
                child.tick().await
            } else {
                // Initialize the child if not already initialized
                if let Some(child) = self.handles.get_mut(child_id) {
                    child.init().await?;
                    self.initialized.insert(child_id.clone());
                    child.tick().await
                } else {
                    return Err(invalid_child_error!(self));
                }
            };

            // Return success as soon as one child succeeds
            if result == Ok(BehaviorStatus::Success) {
                return Ok(BehaviorStatus::Success);
            }
        }
        // If all children fail, return failure
        Ok(BehaviorStatus::Failure)
    }

    async fn init(&mut self) -> Result<BehaviorStatus, BehaviorError> {
        let tree = self.runtime()?.tree.clone();

        // Get all children handles
        for child_id in &self.children {
            let child = tree.get_node(child_id).await;
            if let Some(child) = child {
                self.handles.insert(child_id.clone(), child.clone());
            }
        }
        // Initialize the set for tracking which children are initialized
        self.initialized = HashSet::new();
        Ok(BehaviorStatus::Initialized)
    }

    async fn shutdown(&mut self) {
        // Shutdown all children in reverse order, only if they are initialized
        let mut children = self.children.clone();
        children.reverse();
        for child_id in children {
            if self.initialized.contains(&child_id) {
                if let Some(child) = self.handles.get_mut(&child_id) {
                    child.shutdown().await;
                }
            }
        }
        self.handles.clear();
        self.initialized.clear();
    }

    async fn interrupted(&mut self) {
        for child_id in self.children.iter() {
            if let Some(child) = self.handles.get_mut(&child_id) {
                child.interrupted().await;
            }
        }
    }
}
