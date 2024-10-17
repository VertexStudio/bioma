use bioma_actor::{ActorError, SystemActorError};

/// Enumerates the types of errors that can occur within a behavior tree,
/// such as node type mismatches, tree structure issues, and lifecycle problems.
#[derive(thiserror::Error, Debug)]
pub enum BehaviorError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
}

impl ActorError for BehaviorError {}
