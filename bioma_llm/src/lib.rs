pub mod chat;

pub mod prelude {
    pub use crate::chat::{self, Chat, ChatError, ChatMessages};
    pub use ollama_rs::{
        generation::{
            chat::{ChatMessage, ChatMessageResponse, MessageRole},
            images::Image,
            tools::{ToolCall, ToolInfo},
        },
        models::ModelOptions,
    };
}
