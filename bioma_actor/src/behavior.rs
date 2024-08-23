use crate::actor::ActorId;
use crate::message::{Message, MessageHandler};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Initialize;

#[derive(Clone, Serialize, Deserialize)]
pub struct Run;

#[derive(Clone, Serialize, Deserialize)]
pub struct Interrupted;

#[derive(Clone, Serialize, Deserialize)]
pub struct Shutdown;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Status {
    Standby,
    Running,
    Failure,
    Success,
}

impl Default for Status {
    fn default() -> Self {
        Self::Standby
    }
}

impl Message for Initialize {
    type Response = Status;
}

impl Message for Run {
    type Response = Status;
}

impl Message for Interrupted {
    type Response = ();
}

impl Message for Shutdown {
    type Response = ();
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub enum BehaviorType {
    Action,
    Composite,
    Decorator,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BehaviorNode {
    pub status: Status,
    pub children: Vec<ActorId>,
}

pub trait Behavior: MessageHandler<Message = Run> {
    const TYPE: BehaviorType;

    fn node(&self) -> &BehaviorNode;
    fn node_mut(&mut self) -> &mut BehaviorNode;

    fn status(&self) -> Status {
        self.node().status
    }

    fn set_status(&mut self, status: Status) {
        self.node_mut().status = status;
    }

    fn children(&self) -> &[ActorId] {
        &self.node().children
    }

    fn set_children(&mut self, children: Vec<ActorId>) {
        self.node_mut().children = children;
    }
}

#[macro_export]
macro_rules! impl_behavior_node {
    () => {
        fn node(&self) -> &BehaviorNode {
            &self.node
        }
        fn node_mut(&mut self) -> &mut BehaviorNode {
            &mut self.node
        }
    };
}
