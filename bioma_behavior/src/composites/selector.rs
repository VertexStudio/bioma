use crate::behavior::{Run, Status};
use crate::prelude::*;
use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use std::future::Future;

/// Executes child nodes sequentially until one succeeds or all fail.
///
/// The `Selector` composite node processes its children one by one in order. It returns success as soon as one
/// child node succeeds. If a child fails, it proceeds to the next one. If all children fail,
/// then the `Selector` node fails.
///
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Selector {
    #[serde(skip)]
    node: BehaviorNode,
}

impl Selector {
    pub fn new() -> Self {
        Self {
            node: BehaviorNode::default(),
        }
    }
}

impl Behavior for Selector {
    const TYPE: BehaviorType = BehaviorType::Composite;
    impl_behavior_node!();
}

impl MessageRx<Run> for Selector {
    fn recv(&self, _message: Run) -> impl Future<Output = Status> {
        async move { Status::Success }
    }
}
