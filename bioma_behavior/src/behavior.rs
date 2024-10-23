use crate::tree;
use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
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

    fn tag() -> Cow<'static, str> {
        // Short type name
        let type_name = std::any::type_name::<Self>();
        let type_name = type_name.split("::").last().unwrap();
        type_name.into()
    }
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
    child: Option<ActorId>,
    /// The handle of the child node.
    child_handle: Option<ActorHandle>,
    /// The data of the child node.
    child_data: Option<tree::Node>,
}

impl Decorator {
    pub fn new() -> Self {
        Self { child: None, child_handle: None, child_data: None }
    }

    pub fn copy_child(&mut self, node: &tree::DecoratorNode) {
        self.child_data = node.child.as_ref().map(|boxed_node| (**boxed_node).clone());
    }

    /// Spawns or retrieves the child of this decorator node.
    ///
    /// This method either returns the existing child if it has already been spawned,
    /// or spawns a new child based on the stored child data.
    ///
    /// # Spawning Process
    /// The spawning process works by using the factory of registered actor types:
    /// 1. It retrieves the actor tag from the child data.
    /// 2. It uses the `ActorTagRegistry` to find the corresponding factory for that tag.
    /// 3. The factory then creates and spawns the actor with the provided configuration.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The actor context, which provides access to the engine and registry.
    /// * `options` - Spawn options for creating a new child actor.
    ///
    /// # Returns
    ///
    /// A `Future` that resolves to a `Result` containing:
    /// - `Some(ActorId)` if a child exists or was successfully spawned.
    /// - `None` if there's no child data to spawn from.
    /// - `Err(SystemActorError)` if spawning fails.
    ///
    /// # Note
    ///
    /// If a child is successfully spawned, its handle is stored in `self.child_handle`.
    pub fn child<'a, T: Actor>(
        &'a mut self,
        ctx: &mut ActorContext<T>,
        options: SpawnOptions,
    ) -> impl Future<Output = Result<Option<ActorId>, SystemActorError>> + 'a {
        let child_id = self.child.clone();
        let child_data = self.child_data.clone();
        let registry = ctx.engine().registry().clone();
        let ctx_id = ctx.id().clone();
        let engine = ctx.engine().clone();

        async move {
            if let Some(child_id) = child_id {
                Ok(Some(child_id))
            } else if let Some(child_data) = child_data {
                let child_id = child_data.id(Some(&ctx_id));
                let child_tag = child_data.data().tag.clone();
                let child_config = child_data.value();
                let child_handle = registry.spawn(child_tag, engine, child_config, child_id.clone(), options).await?;
                self.child_handle = Some(child_handle);
                Ok(Some(child_id))
            } else {
                Ok(None)
            }
        }
    }

    /// Stops the child of this decorator node.
    ///
    /// This method removes the child's handle and ID from the decorator node's internal fields.
    /// By releasing the handle, we expect the child and all its descendants to be stopped by the tokio runtime.
    ///
    /// # Note
    ///
    /// This method clears both the `child_handle` and `child` fields, effectively
    /// disconnecting the decorator from its child node.
    pub fn child_stop(&mut self) {
        self.child_handle = None;
        self.child = None;
    }
}

/// Represents a Composite node in a behavior tree.
///
/// Composite nodes can have multiple children and define how these children are executed.
#[derive(Default, Debug)]
pub struct Composite {
    /// A vector of ActorIds representing the children of this composite node.
    children: Vec<ActorId>,
    /// A vector of handles for the children of this composite node.
    children_handles: Vec<ActorHandle>,
    /// A vector of child data for the children of this composite node.
    children_data: Vec<tree::Node>,
}

impl Composite {
    pub fn new() -> Self {
        Self { children: Vec::new(), children_handles: Vec::new(), children_data: Vec::new() }
    }

    pub fn copy_children(&mut self, node: &tree::CompositeNode) {
        self.children_data = node.children.clone();
    }

    /// Spawns or retrieves the children of this composite node.
    ///
    /// This method either returns the existing children if they have already been spawned,
    /// or spawns new children based on the stored child data.
    ///
    /// # Spawning Process
    /// The spawning process works by using the factory of registered actor types:
    /// 1. It retrieves the actor tag from the child data.
    /// 2. It uses the `ActorTagRegistry` to find the corresponding factory for that tag.
    /// 3. The factory then creates and spawns the actor with the provided configuration.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The actor context, which provides access to the engine and registry.
    /// * `options` - Spawn options for creating new child actors.
    ///
    /// # Returns
    ///
    /// A `Future` that resolves to a `Result` containing a vector of `ActorId`s for the children,
    /// or a `SystemActorError` if spawning fails.
    pub fn children<'a, T: Actor>(
        &'a mut self,
        ctx: &mut ActorContext<T>,
        options: SpawnOptions,
    ) -> impl Future<Output = Result<Vec<ActorId>, SystemActorError>> + 'a {
        let children = self.children.clone();
        let children_data = self.children_data.clone();
        let registry = ctx.engine().registry().clone();
        let ctx_id = ctx.id().clone();
        let engine = ctx.engine().clone();

        async move {
            let mut result = Vec::new();

            if !children.is_empty() {
                result = children;
            } else {
                for child_data in children_data {
                    let child_id = child_data.id(Some(&ctx_id));
                    let child_tag = child_data.data().tag.clone();
                    let child_config = child_data.value();
                    let child_handle = registry
                        .spawn(child_tag, engine.clone(), child_config, child_id.clone(), options.clone())
                        .await?;
                    self.children_handles.push(child_handle);
                    result.push(child_id);
                }
                self.children = result.clone();
            }

            Ok(result)
        }
    }

    /// Stops a child of this composite node by its index.
    ///
    /// This method removes the child's handle and ID from the composite node's internal lists.
    /// By releasing the handle, we expect the child and all its children to be stopped by the tokio runtime.
    ///
    /// # Arguments
    ///
    /// * `idx` - The index of the child to stop.
    ///
    /// # Note
    ///
    /// If the provided index is out of bounds for either the handles or children lists,
    /// the method will silently do nothing for that list.
    pub fn child_stop(&mut self, idx: usize) {
        if idx < self.children_handles.len() {
            self.children_handles.remove(idx);
        }
        if idx < self.children.len() {
            self.children.remove(idx);
        }
    }

    pub fn num_children(&self) -> usize {
        self.children.len().max(self.children_data.len())
    }
}
