use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

/// Represents a behavior in a behavior tree.
///
/// This trait defines the core functionality for behaviors in a behavior tree system.
/// Implementors of this trait can be used as nodes in a behavior tree.
pub trait Behavior: Sized + Message<BehaviorTick> {
    /// Returns the node structure of this behavior.
    ///
    /// This method provides the actual node structure, which includes both the type
    /// (Action, Decorator, or Composite) and its contents (no child, single child,
    /// or multiple children, depending on the type).
    ///
    /// # Returns
    ///
    /// A `Node` enum representing the complete structure of this behavior node,
    /// including its type and child nodes (if any).
    fn node(&self) -> Node;
}

/// A message used to trigger the execution of a behavior in a behavior tree.
///
/// This struct is typically sent to behavior nodes to initiate their processing
/// during a behavior tree traversal.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BehaviorTick;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BehaviorCancel;

/// Represents the final status of a behavior after execution.
///
/// Due to the asynchronous nature of behavior execution, behaviors that haven't
/// returned a status yet are implicitly considered to be still running. This enum
/// only represents the final, conclusive states of a behavior.
///
/// In an asynchronous context:
/// - A behavior that hasn't completed its execution is considered "running".
/// - Once a behavior completes, it will return either `Success` or `Failure`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum BehaviorStatus {
    /// The behavior has completed successfully.
    Success,
    /// The behavior has failed to complete.
    Failure,
}

/// Represents a node in a behavior tree.
///
/// This enum defines the three types of nodes that can exist in a behavior tree:
/// Action, Decorator, and Composite.
pub enum Node<'a> {
    /// An Action node, which represents a leaf node in the behavior tree.
    /// Actions are responsible for performing specific tasks.
    Action(&'a Action),

    /// A Decorator node, which has a single child and can modify the behavior
    /// of its child node in some way.
    Decorator(&'a Decorator),

    /// A Composite node, which can have multiple children and defines how
    /// these children are executed (e.g., in sequence or in parallel).
    Composite(&'a Composite),
}

/// Represents an Action node in a behavior tree.
///
/// Action nodes are leaf nodes that perform specific tasks when executed.
#[derive(Default, Debug)]
pub struct Action {}

/// Represents a Decorator node in a behavior tree.
///
/// Decorator nodes have a single child and can modify the behavior of their child node.
#[derive(Default, Debug)]
pub struct Decorator {
    /// The ActorId of the child node.
    pub child: Option<ActorId>,
}

/// Represents a Composite node in a behavior tree.
///
/// Composite nodes can have multiple children and define how these children are executed.
#[derive(Default, Debug)]
pub struct Composite {
    /// A vector of ActorIds representing the children of this composite node.
    pub children: Vec<ActorId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NodeType {
    Action,
    Decorator,
    Composite,
}

impl<'a> Node<'a> {
    pub fn node_type(&self) -> NodeType {
        match self {
            Node::Action(_) => NodeType::Action,
            Node::Decorator(_) => NodeType::Decorator,
            Node::Composite(_) => NodeType::Composite,
        }
    }
}
