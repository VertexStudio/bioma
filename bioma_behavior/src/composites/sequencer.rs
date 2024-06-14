use crate::prelude::*;
use async_trait::async_trait;
use indexmap::IndexSet;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Executes child nodes sequentially, requiring each to succeed before proceeding.
///
/// The `Sequencer` composite node evaluates its children in order, one by one. It returns failure and stops processing
/// as soon as any child fails. If all children succeed in sequence, the `Sequencer` node returns success.
///
#[derive(Serialize, Deserialize)]
pub struct Sequencer {
    pub children: IndexSet<BehaviorId>,

    #[serde(skip)]
    initialized: HashSet<BehaviorId>,
    #[serde(skip)]
    handles: HashMap<BehaviorId, BehaviorHandle>,
    #[serde(skip)]
    runtime: Option<BehaviorRuntime>,
}

impl Sequencer {
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
#[typetag::serde(name = "bioma::core::Sequencer")]
impl BehaviorNode for Sequencer {
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
                // Initialize the child if it hasn't been initialized yet
                if let Some(child) = self.handles.get_mut(child_id) {
                    child.init().await?;
                    self.initialized.insert(child_id.clone());
                    child.tick().await
                } else {
                    return Err(invalid_child_error!(self));
                }
            };

            // Return failure as soon as one child fails
            if result != Ok(BehaviorStatus::Success) {
                return result;
            }
        }
        Ok(BehaviorStatus::Success)
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
