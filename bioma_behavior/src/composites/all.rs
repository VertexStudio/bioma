use crate::prelude::*;
use async_trait::async_trait;
use indexmap::IndexSet;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Executes all child nodes in parallel and succeeds only if all succeed.
///
/// The `All` composite node runs each of its child nodes concurrently. If any child node fails, the `All` node
/// immediately fails and all other child nodes are interrupted; otherwise, it succeeds once all
/// child nodes have successfully completed.
///
#[derive(Serialize, Deserialize)]
pub struct All {
    pub children: IndexSet<BehaviorId>,

    #[serde(skip)]
    handles: HashMap<BehaviorId, BehaviorHandle>,
    #[serde(skip)]
    runtime: Option<BehaviorRuntime>,
}

impl All {
    pub fn new(children: IndexSet<BehaviorId>) -> Box<Self> {
        Box::new(Self {
            children,
            handles: HashMap::new(),
            runtime: None,
        })
    }
}

#[async_trait]
#[typetag::serde(name = "bioma::core::All")]
impl BehaviorNode for All {
    impl_behavior_node_composite!();

    async fn tick(&mut self) -> Result<BehaviorStatus, BehaviorError> {
        // Collect all children
        let mut children = vec![];
        for child_id in &self.children {
            let child = if let Some(child) = self.handles.get_mut(child_id) {
                child.clone()
            } else {
                return Err(invalid_child_error!(self));
            };
            children.push(child);
        }

        // Collect all child tick futures
        let mut child_futures = vec![];
        for child in &mut children {
            let child2 = child.clone();
            let child_future = child.tick();
            child_futures.push(async {
                let result = child_future.await;
                (child2, result)
            });
        }

        // Check all futures and return failure if any child fails
        // Return failure as soon as any child fails
        let mut pinned_futures: Vec<std::pin::Pin<Box<_>>> =
            child_futures.into_iter().map(Box::pin).collect();
        while pinned_futures.len() > 0 {
            let ((handle, status), _, running_futures) =
                futures::future::select_all(pinned_futures).await;
            pinned_futures = running_futures;
            if status == Ok(BehaviorStatus::Failure) {
                drop(pinned_futures);
                // Propagate children interrupted
                for (child_id, child) in self.handles.iter_mut() {
                    if child_id != handle.id() {
                        child.interrupted().await;
                    }
                }
                return Ok(BehaviorStatus::Failure);
            }
        }

        // If all children suceed, return success
        Ok(BehaviorStatus::Success)
    }

    async fn init(&mut self) -> Result<BehaviorStatus, BehaviorError> {
        let tree = self.runtime()?.tree.clone();

        // Get all children handles, and initialize them
        for child_id in &self.children {
            let mut child = tree.get_node(child_id).await;
            if let Some(child) = child.as_mut() {
                child.init().await?;
                self.handles.insert(child_id.clone(), child.clone());
            }
        }
        Ok(BehaviorStatus::Initialized)
    }

    async fn shutdown(&mut self) {
        // Shutdown all children in reverse order
        let mut children = self.children.clone();
        children.reverse();
        for child_id in children.iter() {
            if let Some(child) = self.handles.get_mut(&child_id) {
                child.shutdown().await;
            }
        }
        self.handles.clear();
    }

    async fn interrupted(&mut self) {
        for child_id in self.children.iter() {
            if let Some(child) = self.handles.get_mut(&child_id) {
                child.interrupted().await;
            }
        }
    }
}
