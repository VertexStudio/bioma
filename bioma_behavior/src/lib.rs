pub use behavior::{
    behavior_telemetry, BehaviorHandle, BehaviorId, BehaviorName, BehaviorNode, BehaviorRuntime,
    BehaviorStatus, BehaviorTelemetry, BehaviorTelemetryInfo, BehaviorTree, BehaviorTreeConfig,
    BehaviorTreeHandle, BehaviorTreeId, BehaviorType, DefaultBehaviorTreeConfig,
};

pub use blackboard::{
    Blackboard, BlackboardChannel, BlackboardHandle, BlackboardPublisher, BlackboardSubscriber,
};

pub use error::BehaviorError;

pub use indexmap::{IndexMap, IndexSet};

/// Module for action behavior nodes in a behavior tree.
///
/// Action nodes are behavior nodes that perform specific tasks or operations. These are the leaf nodes of
/// a behavior tree, directly affecting the system or environment. Unlike composite nodes that manage child nodes,
/// action nodes directly execute behavior without further delegation.
///
pub mod actions;

/// Module for composite behavior nodes in a behavior tree.
///
/// Composite nodes are a type of behavior node that manage one or more child nodes. They are used to control
/// the execution flow of their children, making decisions based on the children's return statuses. Composite nodes
/// play a crucial role in structuring the behavior tree, allowing for complex behaviors to be built from simpler,
/// reusable components.
///
pub mod composites;

/// Module for decorator behavior nodes in a behavior tree.
///
/// Decorator nodes are a type of behavior node that modify the control flow or the results of their single child nodes.
/// They do not manage multiple children like composite nodes but instead focus on altering or enhancing the behavior of
/// one child node based on specific conditions or logic.
///
pub mod decorators;

/// Blackboard for sharing data between behavior nodes.
/// 
/// The blackboard is a shared data structure that allows behavior nodes to communicate and share information with each other.
/// Holds pubsub channels for sending and receiving data between behavior nodes.
/// 
pub mod blackboard;

pub mod serializer;

mod behavior;
mod error;

pub mod prelude {
    pub use crate::{
        behavior_telemetry, impl_behavior_node, impl_behavior_node_action,
        impl_behavior_node_composite, impl_behavior_node_decorator, impl_behavior_node_inputs,
        impl_behavior_node_outputs, invalid_child_error, BehaviorError, BehaviorHandle, BehaviorId,
        BehaviorName, BehaviorNode, BehaviorRuntime, BehaviorStatus, BehaviorTelemetry,
        BehaviorTelemetryInfo, BehaviorTree, BehaviorTreeConfig, BehaviorTreeHandle,
        BehaviorTreeId, BehaviorType, Blackboard, BlackboardChannel, BlackboardHandle,
        BlackboardPublisher, BlackboardSubscriber, DefaultBehaviorTreeConfig, IndexSet,
    };
}
