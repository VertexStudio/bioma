use std::borrow::Cow;

/// Enumerates the types of errors that can occur in Actor framework
#[derive(thiserror::Error, Debug)]
pub enum ActorError {
    // IO error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Engine error: {0}")]
    EngineError(#[from] surrealdb::Error),
    #[error("MessageTypeMismatch: {0}")]
    MessageTypeMismatch(&'static str),
    #[error("LiveStream error: {0}")]
    LiveStream(Cow<'static, str>),
    // json_serde::Error
    #[error("JsonSerde error: {0}")]
    JsonSerde(#[from] serde_json::Error),
    // Id mismatch a and b
    #[error("Id mismatch: {0:?} {1:?}")]
    IdMismatch(surrealdb::sql::Thing, surrealdb::sql::Thing),
    #[error("Actor kind mismatch: {0} {1}")]
    ActorKindMismatch(Cow<'static, str>, Cow<'static, str>),
}
