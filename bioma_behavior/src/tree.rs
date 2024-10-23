use crate::behavior::{self, Behavior, BehaviorTick};
use crate::error::BehaviorError;
use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use tokio::sync::oneshot;
use tracing::debug;

/// Behavior tree node type designed to be ergonomic and easy to view and edit in json.
/// Any weirdness is due to the need to serialize/deserialize the node type as part of the node definition.
/// Custom behavior properties are kept under the `config` field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum Node {
    /// An action node in the behavior tree.
    Action(ActionNode),
    /// A decorator node in the behavior tree.
    Decorator(DecoratorNode),
    /// A composite node in the behavior tree.
    Composite(CompositeNode),
}

/// Represents an action node in the behavior tree.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActionNode {
    /// The data associated with this action node.
    #[serde(flatten)]
    pub data: NodeData,
}

/// Represents a decorator node in the behavior tree.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecoratorNode {
    /// The data associated with this decorator node.
    #[serde(flatten)]
    pub data: NodeData,
    /// The optional child node of this decorator.
    pub child: Option<Box<Node>>,
}

/// Represents a composite node in the behavior tree.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompositeNode {
    /// The data associated with this composite node.
    #[serde(flatten)]
    pub data: NodeData,
    /// The children nodes of this composite.
    pub children: Vec<Node>,
}

/// Common data shared by all node types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeData {
    /// The tag associated with this node.
    pub tag: Cow<'static, str>,
    /// The unique identifier for this node.
    pub uid: Cow<'static, str>,
    /// The configuration data for this node.
    pub config: serde_json::Value,
}

impl NodeData {
    /// Generates an `ActorId` for this node.
    ///
    /// # Arguments
    ///
    /// * `prefix_id` - An optional prefix `ActorId` to include in the generated ID.
    ///
    /// # Returns
    ///
    /// An `ActorId` for this node.
    pub fn id(&self, prefix_id: Option<&ActorId>) -> ActorId {
        if let Some(prefix_id) = prefix_id {
            ActorId::with_tag(format!("{}/{}", prefix_id.name(), self.uid.clone()), self.tag.clone())
        } else {
            ActorId::with_tag(self.uid.clone(), self.tag.clone())
        }
    }
}

impl Node {
    /// Creates a new `Node` from a given behavior and its children.
    ///
    /// # Arguments
    ///
    /// * `uid` - The unique identifier for the node.
    /// * `node` - The behavior implementation.
    /// * `children` - The child nodes (if any).
    ///
    /// # Returns
    ///
    /// A `Result` containing the new `Node` or a `BehaviorError`.
    pub fn from<T: Behavior>(
        uid: impl Into<Cow<'static, str>>,
        node: T,
        children: Vec<Node>,
    ) -> Result<Self, BehaviorError> {
        let data = serde_json::to_value(&node).unwrap();
        let tag = T::tag();
        match node.node().node_type() {
            behavior::NodeType::Action => {
                if children.len() > 0 {
                    panic!("Action nodes cannot have children");
                }
                Ok(Node::Action(ActionNode { data: NodeData { tag: tag.into(), uid: uid.into(), config: data } }))
            }
            behavior::NodeType::Decorator => {
                if children.len() > 1 {
                    panic!("Decorator nodes can have only one child");
                }
                let child = children.first().cloned().map(Box::new);
                Ok(Node::Decorator(DecoratorNode {
                    data: NodeData { tag: tag.into(), uid: uid.into(), config: data },
                    child,
                }))
            }
            behavior::NodeType::Composite => Ok(Node::Composite(CompositeNode {
                data: NodeData { tag: tag.into(), uid: uid.into(), config: data },
                children,
            })),
        }
    }

    /// Returns a reference to the `NodeData` of this node.
    pub fn data(&self) -> &NodeData {
        match self {
            Node::Action(node) => &node.data,
            Node::Decorator(node) => &node.data,
            Node::Composite(node) => &node.data,
        }
    }

    /// Returns the serialized value of this node.
    pub fn value(&self) -> serde_json::Value {
        match self {
            Node::Action(node) => serde_json::to_value(&node).unwrap(),
            Node::Decorator(node) => serde_json::to_value(&node).unwrap(),
            Node::Composite(node) => serde_json::to_value(&node).unwrap(),
        }
    }

    /// Returns a vector of child nodes for composite nodes, or an empty vector for other node types.
    pub fn children(&self) -> Vec<Node> {
        match self {
            Node::Composite(node) => node.children.clone(),
            _ => vec![],
        }
    }

    /// Returns the child node for decorator nodes, or None for other node types.
    pub fn child(&self) -> Option<Node> {
        match self {
            Node::Decorator(node) => node.child.clone().map(|c| c.as_ref().clone()),
            _ => None,
        }
    }

    /// Generates an `ActorId` for this node.
    ///
    /// # Arguments
    ///
    /// * `prefix_id` - An optional prefix `ActorId` to include in the generated ID.
    ///
    /// # Returns
    ///
    /// An `ActorId` for this node.
    pub fn id(&self, prefix_id: Option<&ActorId>) -> ActorId {
        self.data().id(prefix_id)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BehaviorTree {
    pub root: Node,
    pub logs: Vec<String>,
    #[serde(skip)]
    pub root_handle: Option<ActorHandle>,
}

impl PartialEq for BehaviorTree {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root
    }
}

impl Actor for BehaviorTree {
    type Error = BehaviorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        let (tx, mut rx) = oneshot::channel();
        let root_id = self.root.data().id(Some(&ctx.id()));
        let root_tag = self.root.data().tag.clone();
        let root_config = self.root.value();
        let registry = ctx.engine().registry();
        let root_handle = registry
            .spawn(root_tag, ctx.engine().clone(), root_config, root_id.clone(), SpawnOptions::default())
            .await?;

        debug!("BehaviorTree::start {}", ctx.id());

        let root_handle: tokio::task::JoinHandle<Result<(), SystemActorError>> = tokio::spawn(async move {
            let res = root_handle.await?;
            let _ = tx.send(res);
            Ok(())
        });
        self.root_handle = Some(root_handle);

        // Send a tick to the root
        let _ = ctx.do_send_as(BehaviorTick, &root_id).await;

        let mut stream = ctx.recv().await?;
        let result = Ok(());

        loop {
            tokio::select! {
                Some(Ok(_frame)) = stream.next() => {
                    // Handle the frame - continue loop after processing
                },
                root_result = &mut rx => {
                    match root_result {
                        Ok(_) => break,
                        Err(_) => break,
                    }
                }
            }
        }

        debug!("BehaviorTree::start: end {}", ctx.id());

        result
    }
}

impl BehaviorTree {}
