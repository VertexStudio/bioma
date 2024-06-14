use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum KnowledgeError {
    #[error("Initialization error: {0}")]
    InitializationError(String),
}
