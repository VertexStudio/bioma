use crate::{BehaviorId, BehaviorName, BehaviorStatus};
use std::borrow::Cow;
use thiserror::Error;
use std::path::PathBuf;

/// Enumerates the types of errors that can occur within a behavior tree,
/// such as node type mismatches, tree structure issues, and lifecycle problems.
#[derive(Error, Debug, PartialEq, Clone)]
pub enum BehaviorError {
    #[error("Invalid node type at: {0}({1})")]
    BehaviorNodeTypeError(BehaviorName, BehaviorId),
    #[error("Invalid behavior tree at: {0}({1})")]
    InvalidTree(BehaviorName, BehaviorId),
    #[error("Invalid child at: {0}({1})")]
    InvalidChild(BehaviorName, BehaviorId),
    #[error("Invalid status at: {0}({1}), {2} -> {3}")]
    InvalidStatus(BehaviorName, BehaviorId, BehaviorStatus, BehaviorStatus),
    #[error("Invalid root id: {0}")]
    InvalidRoot(BehaviorId),
    #[error("Runtime not initialize")]
    RuntimeNotInitialized,
    #[error("Invalid blackboard channel: {0}")]
    InvalidBlackboardChannel(Cow<'static, str>),
    #[error("Behavior tree parse error: {0}")]
    ParseError(Cow<'static, str>),
    #[error("Behavior tree load error: {0}")]
    LoadError(PathBuf),
}

#[macro_export]
macro_rules! invalid_child_error {
    ($self:ident) => {
        BehaviorError::InvalidChild(
            $self.name(),
            if let Ok(node) = $self.runtime() {
                node.id.clone()
            } else {
                BehaviorId::new("unknown")
            },
        )
    };
}
