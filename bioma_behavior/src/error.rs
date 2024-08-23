use std::borrow::Cow;

/// Enumerates the types of errors that can occur within a behavior tree,
/// such as node type mismatches, tree structure issues, and lifecycle problems.
#[derive(thiserror::Error, Debug)]
pub enum BehaviorError {
    #[error("Runtime not initialize")]
    RuntimeNotInitialized,
    #[error("Invalid blackboard channel: {0}")]
    InvalidBlackboardChannel(Cow<'static, str>),
    #[error("Behavior tree parse error: {0}")]
    ParseError(Cow<'static, str>),
}

