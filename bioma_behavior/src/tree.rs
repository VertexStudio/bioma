use crate::behavior::{self, Behavior};
use crate::error::BehaviorError;
use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    tag: Cow<'static, str>,
    uid: Cow<'static, str>,
    config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionNode {
    #[serde(flatten)]
    common: NodeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecoratorNode {
    #[serde(flatten)]
    common: NodeConfig,
    child: Box<Node>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompositeNode {
    #[serde(flatten)]
    common: NodeConfig,
    children: Vec<Node>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Node {
    Action(ActionNode),
    Decorator(DecoratorNode),
    Composite(CompositeNode),
}

impl Node {
    pub fn action<T: Behavior>(
        tag: impl Into<Cow<'static, str>>,
        uid: impl Into<Cow<'static, str>>,
        config: &T,
    ) -> Self {
        let config_value = serde_json::to_value(config).unwrap();
        match config.node().node_type() {
            behavior::NodeType::Action => Node::Action(ActionNode {
                common: NodeConfig { tag: tag.into(), uid: uid.into(), config: config_value },
            }),
            behavior::NodeType::Decorator => panic!("Invalid node type, expected Action, got Decorator"),
            behavior::NodeType::Composite => panic!("Invalid node type, expected Action, got Composite"),
        }
    }

    pub fn decorator<T: Behavior>(
        tag: impl Into<Cow<'static, str>>,
        uid: impl Into<Cow<'static, str>>,
        config: &T,
        child: Node,
    ) -> Self {
        let config_value = serde_json::to_value(config).unwrap();
        match config.node().node_type() {
            behavior::NodeType::Action => panic!("Invalid node type, expected Decorator, got Action"),
            behavior::NodeType::Decorator => Node::Decorator(DecoratorNode {
                common: NodeConfig { tag: tag.into(), uid: uid.into(), config: config_value },
                child: Box::new(child),
            }),
            behavior::NodeType::Composite => panic!("Invalid node type, expected Decorator, got Composite"),
        }
    }

    pub fn composite<T: Behavior>(
        tag: impl Into<Cow<'static, str>>,
        uid: impl Into<Cow<'static, str>>,
        config: &T,
        children: Vec<Node>,
    ) -> Self {
        let config_value = serde_json::to_value(config).unwrap();
        match config.node().node_type() {
            behavior::NodeType::Action => panic!("Invalid node type, expected Composite, got Action"),
            behavior::NodeType::Decorator => panic!("Invalid node type, expected Composite, got Decorator"),
            behavior::NodeType::Composite => Node::Composite(CompositeNode {
                common: NodeConfig { tag: tag.into(), uid: uid.into(), config: config_value },
                children,
            }),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BehaviorTree {
    pub root: Option<Node>,
    #[serde(skip)]
    pub handles: HashMap<ActorId, Option<ActorHandle>>,
}

impl Actor for BehaviorTree {
    type Error = BehaviorError;

    async fn start(&mut self, _ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        // let mut stream = ctx.recv().await?;
        // while let Some(Ok(frame)) = stream.next().await {}
        Ok(())
    }
}
