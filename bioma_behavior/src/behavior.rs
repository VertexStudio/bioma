use crate::{BehaviorError, BlackboardChannel, BlackboardHandle};
use as_any::AsAny;
use async_trait::async_trait;
use derive_more::Display;
use indexmap::{IndexMap, IndexSet};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::VecDeque;
use std::fmt::Write;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, MutexGuard};

/// Represents a unique identifier for behavior nodes, encapsulated as a string.
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct BehaviorId(Cow<'static, str>);

impl Default for BehaviorId {
    fn default() -> Self {
        panic!("BehaviorId::default() not implemented");
    }
}

impl BehaviorId {
    pub fn new(id: impl Into<Cow<'static, str>>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for BehaviorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Represents a human-readable name for a behavior node.
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct BehaviorName(&'static str);

impl std::fmt::Display for BehaviorName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Enumerates different types of behavior nodes, such as actions, composites, or decorators.
pub enum BehaviorType {
    Action,
    Composite,
    Decorator,
}

/// Defines the core functionality for all behavior nodes within a behavior tree,
/// including lifecycle methods and utility functions for node management.
#[async_trait]
#[typetag::serde(tag = "type")]
pub trait BehaviorNode: Send + Sync + AsAny {
    async fn tick(&mut self) -> Result<BehaviorStatus, BehaviorError>;
    async fn init(&mut self) -> Result<BehaviorStatus, BehaviorError> {
        Ok(BehaviorStatus::Initialized)
    }
    async fn shutdown(&mut self) {}
    async fn interrupted(&mut self) {}
    fn name(&self) -> BehaviorName {
        BehaviorName(self.typetag_name())
    }
    fn inputs(&self) -> IndexSet<BlackboardChannel> {
        IndexSet::new()
    }
    fn outputs(&self) -> IndexSet<BlackboardChannel> {
        IndexSet::new()
    }
    fn children(&self) -> IndexSet<BehaviorId> {
        panic!("BehaviorNode::children() not implemented");
    }
    fn node_type(&self) -> BehaviorType {
        panic!("BehaviorNode::node_type() not implemented");
    }
    fn runtime(&self) -> Result<&BehaviorRuntime, BehaviorError> {
        panic!("BehaviorNode::runtime() not implemented");
    }
    fn runtime_mut(&mut self) -> Result<&mut BehaviorRuntime, BehaviorError> {
        panic!("BehaviorNode::runtime_mut() not implemented");
    }
    fn set_runtime(&mut self, _runtime: BehaviorRuntime) {
        panic!("BehaviorNode::set_runtime() not implemented");
    }

    fn telemetry_info(&self) -> Result<BehaviorTelemetryInfo, BehaviorError> {
        let runtime = self.runtime()?;
        Ok(BehaviorTelemetryInfo {
            tree_id: Some(runtime.tree.id().clone()),
            node_name: Some(self.name()),
            node_id: Some(runtime.id.clone()),
            telemetry_tx: runtime.telemetry_tx.clone(),
        })
    }
}

/// Holds runtime information for a behavior node, including status, telemetry, and tree structure.
pub struct BehaviorRuntime {
    pub id: BehaviorId,
    pub tree: BehaviorTreeHandle,
    pub status: Result<BehaviorStatus, BehaviorError>,
    pub telemetry_tx: Option<tokio::sync::mpsc::Sender<BehaviorTelemetry>>,
    pub blackboard: Option<BlackboardHandle>,
}

/// Sends telemetry data for behavior nodes during their execution, supporting runtime diagnostics and monitoring.
pub async fn behavior_telemetry(
    node: &dyn BehaviorNode,
    status: Result<BehaviorStatus, BehaviorError>,
    log: Option<String>,
) {
    if let Ok(runtime) = node.runtime() {
        if let Some(telemetry_tx) = &runtime.telemetry_tx {
            let telemetry = BehaviorTelemetry {
                tree_id: Some(runtime.tree.id().clone()),
                name: Some(node.name()),
                node_id: Some(runtime.id.clone()),
                status: Some(status.clone()),
                log: log.map(|l| l.into()),
            };
            let _ = telemetry_tx.send(telemetry).await;
        }
    }
}

#[macro_export]
macro_rules! impl_behavior_node_inputs {
    () => {
        fn inputs(&self) -> IndexSet<BlackboardChannel> {
            self.inputs.clone()
        }
    };
}

#[macro_export]
macro_rules! impl_behavior_node_outputs {
    () => {
        fn outputs(&self) -> IndexSet<BlackboardChannel> {
            self.outputs.clone()
        }
    };
}

#[macro_export]
macro_rules! impl_behavior_node {
    () => {
        fn runtime(&self) -> Result<&BehaviorRuntime, BehaviorError> {
            if let Some(runtime) = &self.runtime {
                Ok(runtime)
            } else {
                Err(BehaviorError::RuntimeNotInitialized)
            }
        }

        fn runtime_mut(&mut self) -> Result<&mut BehaviorRuntime, BehaviorError> {
            if let Some(runtime) = &mut self.runtime {
                Ok(runtime)
            } else {
                Err(BehaviorError::RuntimeNotInitialized)
            }
        }

        fn set_runtime(&mut self, runtime: BehaviorRuntime) {
            self.runtime = Some(runtime);
        }
    };
}

#[macro_export]
macro_rules! impl_behavior_node_action {
    () => {
        impl_behavior_node!();

        fn children(&self) -> IndexSet<BehaviorId> {
            IndexSet::new()
        }

        fn node_type(&self) -> BehaviorType {
            BehaviorType::Action
        }
    };
}

#[macro_export]
macro_rules! impl_behavior_node_composite {
    () => {
        impl_behavior_node!();

        fn children(&self) -> IndexSet<BehaviorId> {
            self.children.clone()
        }

        fn node_type(&self) -> BehaviorType {
            BehaviorType::Composite
        }
    };
}

#[macro_export]
macro_rules! impl_behavior_node_decorator {
    () => {
        impl_behavior_node!();

        fn children(&self) -> IndexSet<BehaviorId> {
            if let Some(child) = &self.child {
                IndexSet::from_iter(vec![child.clone()])
            } else {
                IndexSet::new()
            }
        }

        fn node_type(&self) -> BehaviorType {
            BehaviorType::Decorator
        }
    };
}

/// Enumerates possible states or outcomes for behavior node execution, such as success, failure, or running.
#[derive(Debug, Display, PartialEq, Clone, Copy)]
pub enum BehaviorStatus {
    Shutdown,
    Initialized,
    Success,
    Failure,
    Running,
    Interrupted,
}

/// Manages the lifecycle and execution of a behavior node, ensuring thread safety and access to runtime information.
#[derive(Clone)]
pub struct BehaviorHandle {
    tree_id: BehaviorTreeId,
    node_id: BehaviorId,
    node: Arc<Mutex<Box<dyn BehaviorNode>>>,
    name: BehaviorName,
    telemetry_tx: Option<mpsc::Sender<BehaviorTelemetry>>,
    ticks: usize,
}

impl BehaviorHandle {
    pub fn boxed(
        tree_id: &BehaviorTreeId,
        node_id: &BehaviorId,
        node: Box<dyn BehaviorNode>,
        telemetry_tx: Option<mpsc::Sender<BehaviorTelemetry>>,
    ) -> Self {
        BehaviorHandle {
            tree_id: tree_id.clone(),
            node_id: node_id.clone(),
            name: node.name(),
            node: Arc::new(Mutex::new(node)),
            telemetry_tx,
            ticks: 0,
        }
    }

    fn new(
        tree_id: &BehaviorTreeId,
        node_id: &BehaviorId,
        node: Box<dyn BehaviorNode>,
        telemetry_tx: Option<mpsc::Sender<BehaviorTelemetry>>,
    ) -> Self {
        BehaviorHandle {
            tree_id: tree_id.clone(),
            node_id: node_id.clone(),
            name: node.name(),
            node: Arc::new(Mutex::new(node)),
            telemetry_tx,
            ticks: 0,
        }
    }

    pub fn id(&self) -> &BehaviorId {
        &self.node_id
    }

    pub fn node(&self) -> Arc<Mutex<Box<dyn BehaviorNode>>> {
        self.node.clone()
    }

    pub async fn tick(&mut self) -> Result<BehaviorStatus, BehaviorError> {
        self.ticks += 1;
        let mut node = self.node.lock().await;
        let runtime = node.runtime_mut()?;

        if let Some(telemetry_tx) = &self.telemetry_tx {
            let telemetry = BehaviorTelemetry {
                tree_id: Some(self.tree_id.clone()),
                name: Some(self.name.clone()),
                node_id: Some(self.node_id.clone()),
                status: Some(runtime.status.clone()),
                log: Some("TickBegin".into()),
            };
            let _ = telemetry_tx.send(telemetry).await;
        }
        runtime.status = Ok(BehaviorStatus::Running);

        let status = node.tick().await;
        let runtime = node.runtime_mut()?;
        runtime.status = status;

        if let Some(telemetry_tx) = &self.telemetry_tx {
            let telemetry = BehaviorTelemetry {
                tree_id: Some(self.tree_id.clone()),
                name: Some(self.name.clone()),
                node_id: Some(self.node_id.clone()),
                status: Some(runtime.status.clone()),
                log: Some("TickEnd".into()),
            };
            let _ = telemetry_tx.send(telemetry).await;
        }
        runtime.status.clone()
    }

    pub async fn init(&mut self) -> Result<BehaviorStatus, BehaviorError> {
        let mut node = self.node.lock().await;
        let runtime = node.runtime()?;

        if let Some(telemetry_tx) = &self.telemetry_tx {
            let telemetry = BehaviorTelemetry {
                tree_id: Some(self.tree_id.clone()),
                name: Some(self.name.clone()),
                node_id: Some(self.node_id.clone()),
                status: Some(runtime.status.clone()),
                log: Some("InitBegin".into()),
            };
            let _ = telemetry_tx.send(telemetry).await;
        }

        let status = node.init().await;
        let runtime = node.runtime_mut()?;
        runtime.status = status;

        if let Some(telemetry_tx) = &self.telemetry_tx {
            let telemetry = BehaviorTelemetry {
                tree_id: Some(self.tree_id.clone()),
                name: Some(self.name.clone()),
                node_id: Some(self.node_id.clone()),
                status: Some(runtime.status.clone()),
                log: Some("InitEnd".into()),
            };
            let _ = telemetry_tx.send(telemetry).await;
        }
        runtime.status.clone()
    }

    pub async fn interrupted(&mut self) {
        let mut node = self.node.lock().await;
        let Ok(runtime) = node.runtime_mut() else {
            return;
        };
        let status = runtime.status.clone();
        // It is only possible to interrupt a running node
        if let Ok(BehaviorStatus::Running) = status {
            if let Some(telemetry_tx) = &self.telemetry_tx {
                let telemetry = BehaviorTelemetry {
                    tree_id: Some(self.tree_id.clone()),
                    name: Some(self.name.clone()),
                    node_id: Some(self.node_id.clone()),
                    status: Some(status.clone()),
                    log: Some("InterruptedBegin".into()),
                };
                let _ = telemetry_tx.send(telemetry);
            }

            node.interrupted().await;
            let Ok(runtime) = node.runtime_mut() else {
                return;
            };
            runtime.status = Ok(BehaviorStatus::Interrupted);

            if let Some(telemetry_tx) = &self.telemetry_tx {
                let telemetry = BehaviorTelemetry {
                    tree_id: Some(self.tree_id.clone()),
                    name: Some(self.name.clone()),
                    node_id: Some(self.node_id.clone()),
                    status: Some(runtime.status.clone()),
                    log: Some("InterruptedEnd".into()),
                };
                let _ = telemetry_tx.send(telemetry);
            }
        }
    }

    pub async fn shutdown(&mut self) {
        let mut node = self.node.lock().await;
        let Ok(runtime) = node.runtime_mut() else {
            return;
        };

        if let Some(telemetry_tx) = &self.telemetry_tx {
            let telemetry = BehaviorTelemetry {
                tree_id: Some(self.tree_id.clone()),
                name: Some(self.name.clone()),
                node_id: Some(self.node_id.clone()),
                status: Some(runtime.status.clone()),
                log: Some("ShutdownBegin".into()),
            };
            let _ = telemetry_tx.send(telemetry).await;
        }

        node.shutdown().await;
        let Ok(runtime) = node.runtime_mut() else {
            return;
        };
        runtime.status = Ok(BehaviorStatus::Shutdown);

        if let Some(telemetry_tx) = &self.telemetry_tx {
            let telemetry = BehaviorTelemetry {
                tree_id: Some(self.tree_id.clone()),
                name: Some(self.name.clone()),
                node_id: Some(self.node_id.clone()),
                status: Some(runtime.status.clone()),
                log: Some("ShutdownEnd".into()),
            };
            let _ = telemetry_tx.send(telemetry).await;
        }
    }

    pub fn name(&self) -> &BehaviorName {
        &self.name
    }

    pub async fn get<NodeType, Output>(
        &self,
        f: impl FnOnce(&NodeType) -> Output,
    ) -> Result<Output, BehaviorError>
    where
        NodeType: BehaviorNode,
    {
        let node = self.node.lock().await;
        let node = node.as_ref();
        let node_id = node.runtime()?.id.clone();
        let node = node
            .as_any()
            .downcast_ref::<NodeType>()
            .ok_or(BehaviorError::BehaviorNodeTypeError(node.name(), node_id))?;
        let res = f(node);
        Ok(res)
    }
}

impl std::fmt::Debug for BehaviorHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}({:?})", self.name(), self.id())
    }
}

/// Provides a configuration interface for behavior trees, allowing different setups for tree behaviors and properties.
#[typetag::serde(tag = "type")]
pub trait BehaviorTreeConfig: Send + Sync + std::fmt::Debug {}

/// Provides a default implementation for a behavior tree configuration, including a basic name and description.
#[derive(Debug, Serialize, Deserialize)]
pub struct DefaultBehaviorTreeConfig {
    pub name: String,
    pub desc: String,
}

#[typetag::serde(name = "bioma::core::DefaultBehaviorTreeConfig")]
impl BehaviorTreeConfig for DefaultBehaviorTreeConfig {}

impl DefaultBehaviorTreeConfig {
    pub fn new(name: &str, desc: &str) -> Box<Self> {
        Box::new(Self {
            name: name.to_string(),
            desc: desc.to_string(),
        })
    }

    pub fn mock() -> Box<Self> {
        Box::new(Self {
            name: "Behavior Tree".to_string(),
            desc: "Async behavior tree system".to_string(),
        })
    }
}

/// Struct for capturing and transmitting telemetry data from behavior nodes, useful for debugging and analysis.
#[derive(Debug, PartialEq, Clone)]
pub struct BehaviorTelemetry {
    pub tree_id: Option<BehaviorTreeId>,
    pub name: Option<BehaviorName>,
    pub node_id: Option<BehaviorId>,
    pub status: Option<Result<BehaviorStatus, BehaviorError>>,
    pub log: Option<Cow<'static, str>>,
}

impl std::fmt::Display for BehaviorTelemetry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tree_id = match &self.tree_id {
            Some(tree_id) => format!("[{}]", tree_id),
            None => "".to_string(),
        };

        let node_name = match &self.name {
            Some(name) => format!(" {}", name),
            None => "".to_string(),
        };

        let node_id = match &self.node_id {
            Some(node_id) => format!("({})", node_id),
            None => "".to_string(),
        };

        let sep = if self.status.is_some() || self.log.is_some() {
            ":"
        } else {
            ""
        };

        let status = match &self.status {
            Some(status) => format!(" {:?}", status),
            None => "".to_string(),
        };

        let log = match &self.log {
            Some(log) => format!(" - {}", log),
            None => "".to_string(),
        };

        write!(f, "{tree_id}{node_name}{node_id}{sep}{status}{log}")
    }
}

impl BehaviorTelemetry {
    pub fn from_static(
        tree_id: &'static str,
        name: &'static str,
        node_id: &'static str,
        status: Option<Result<BehaviorStatus, BehaviorError>>,
        log: Option<&'static str>,
    ) -> Self {
        Self {
            tree_id: Some(BehaviorTreeId::new(tree_id)),
            name: Some(BehaviorName(name)),
            node_id: Some(BehaviorId::new(node_id)),
            status,
            log: log.map(|l| Cow::Borrowed(l)),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct BehaviorTelemetryInfo {
    pub tree_id: Option<BehaviorTreeId>,
    pub node_name: Option<BehaviorName>,
    pub node_id: Option<BehaviorId>,
    pub telemetry_tx: Option<mpsc::Sender<BehaviorTelemetry>>,
}

/// Unique identifier for a behavior tree, encapsulated as a string.
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct BehaviorTreeId(Cow<'static, str>);

impl Default for BehaviorTreeId {
    fn default() -> Self {
        panic!("BehaviorId::default() not implemented");
    }
}

impl BehaviorTreeId {
    pub fn new(id: impl Into<Cow<'static, str>>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for BehaviorTreeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Represents a behavior tree, containing nodes, configuration, and mechanisms for node management and execution.
#[derive(Debug)]
pub struct BehaviorTree {
    id: BehaviorTreeId,
    pub config: Box<dyn BehaviorTreeConfig>,
    pub root: Option<BehaviorId>,
    pub nodes: IndexMap<BehaviorId, BehaviorHandle>,
    pub telemetry_tx: Option<mpsc::Sender<BehaviorTelemetry>>,
    pub blackboard: Option<BlackboardHandle>,
}

impl BehaviorTree {
    pub fn new(
        id: &BehaviorTreeId,
        root: &BehaviorId,
        config: Box<dyn BehaviorTreeConfig>,
        telemetry_tx: Option<mpsc::Sender<BehaviorTelemetry>>,
        blackboard: Option<BlackboardHandle>,
    ) -> Self {
        Self {
            id: id.clone(),
            config: config,
            root: Some(root.clone()),
            nodes: IndexMap::new(),
            telemetry_tx,
            blackboard,
        }
    }

    pub fn id(&self) -> &BehaviorTreeId {
        &self.id
    }

    fn add_node(&mut self, node_id: &BehaviorId, node: Box<dyn BehaviorNode>) {
        let handle = BehaviorHandle::new(&self.id, &node_id, node, self.telemetry_tx.clone());
        self.nodes.insert(node_id.clone(), handle.clone());
    }

    fn get_node(&self, id: &BehaviorId) -> Option<&BehaviorHandle> {
        self.nodes.get(id)
    }

    // Creates an ascii tree
    async fn ascii_tree(&self, root_node_id: &BehaviorId, ascii_tree: &mut String) {
        let mut stack: VecDeque<(BehaviorId, usize, bool)> = VecDeque::new();
        stack.push_back((root_node_id.clone(), 0, true));

        while let Some((node_id, depth, is_last)) = stack.pop_front() {
            if let Some(node) = self.nodes.get(&node_id) {
                let node = node.node.lock().await;
                let runtime = match node.runtime() {
                    Ok(runtime) => runtime,
                    Err(_) => continue,
                };

                let name = node.name();
                let status = match &runtime.status {
                    Ok(status) => format!("{:?}", status),
                    Err(err) => format!("{:?}", err),
                };
                let tree_lines = if depth == 0 {
                    "   "
                } else if is_last {
                    "└─ "
                } else {
                    "├─ "
                };
                let _ = writeln!(
                    ascii_tree,
                    "{:indent$}{}{}({}): {}",
                    "",
                    tree_lines,
                    name,
                    runtime.id,
                    status,
                    indent = depth * 4
                );

                let children = node.children();
                let children_len = children.len();
                for (i, child_id) in children.iter().enumerate().rev() {
                    stack.push_front((child_id.clone(), depth + 1, i == children_len - 1));
                }
            }
        }
    }
}

/// Facilitates interactions with a behavior tree, allowing nodes to be added, modified, or executed, and maintaining a tree-wide state.
#[derive(Clone)]
pub struct BehaviorTreeHandle {
    id: BehaviorTreeId,
    tree: Arc<Mutex<BehaviorTree>>,
    ascii_tree: String,
}

impl BehaviorTreeHandle {
    pub fn new(tree: BehaviorTree) -> Self {
        Self {
            id: tree.id.clone(),
            tree: Arc::new(Mutex::new(tree)),
            ascii_tree: String::new(),
        }
    }

    pub fn id(&self) -> &BehaviorTreeId {
        &self.id
    }

    pub async fn lock(&self) -> MutexGuard<BehaviorTree> {
        self.tree.lock().await
    }

    pub async fn get_node(&self, id: &BehaviorId) -> Option<BehaviorHandle> {
        let tree = self.tree.lock().await;
        tree.get_node(id).cloned()
    }

    pub async fn add_node(&mut self, id: &BehaviorId, mut node: Box<dyn BehaviorNode>) {
        // Prepare runtime for the node
        let mut tree = self.tree.lock().await;
        let runtime = BehaviorRuntime {
            id: id.clone(),
            tree: self.clone(),
            status: Ok(BehaviorStatus::Shutdown),
            telemetry_tx: tree.telemetry_tx.clone(),
            blackboard: tree.blackboard.clone(),
        };
        node.set_runtime(runtime);
        // Add node to the tree
        tree.add_node(id, node);
        // Update the ascii tree
        if let Some(root) = &tree.root {
            self.ascii_tree.clear();
            tree.ascii_tree(root, &mut self.ascii_tree).await;
        }
    }

    pub async fn telemetry_tx(&self) -> Option<mpsc::Sender<BehaviorTelemetry>> {
        self.tree.lock().await.telemetry_tx.clone()
    }

    pub async fn run(&mut self) -> Result<BehaviorStatus, BehaviorError> {
        let tree = self.tree.lock().await;
        let root = tree.root.clone();
        let Some(root) = root else {
            return Err(BehaviorError::InvalidRoot(BehaviorId::new("(None)")));
        };
        let Some(root) = tree.get_node(&root) else {
            return Err(BehaviorError::InvalidRoot(root.clone()));
        };
        let mut root = root.clone();
        drop(tree);
        let _ = root.init().await?;
        let status = root.tick().await?;
        // Update the ascii tree
        let tree = self.tree.lock().await;
        if let Some(root) = &tree.root {
            self.ascii_tree.clear();
            tree.ascii_tree(root, &mut self.ascii_tree).await;
        }
        Ok(status)
    }

    pub async fn shutdown(&mut self) -> Result<BehaviorStatus, BehaviorError> {
        let tree = self.tree.lock().await;
        let root = tree.root.clone();
        let Some(root) = root else {
            return Err(BehaviorError::InvalidRoot(BehaviorId::new("(None)")));
        };
        let Some(root) = tree.get_node(&root) else {
            return Err(BehaviorError::InvalidRoot(root.clone()));
        };
        let mut root = root.clone();
        drop(tree);
        root.shutdown().await;
        // Update the ascii tree
        let tree = self.tree.lock().await;
        if let Some(root) = &tree.root {
            self.ascii_tree.clear();
            tree.ascii_tree(root, &mut self.ascii_tree).await;
        }
        Ok(BehaviorStatus::Shutdown)
    }
}

impl std::fmt::Debug for BehaviorTreeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "---\nbt({})\n{}\n", self.id, self.ascii_tree)
    }
}
