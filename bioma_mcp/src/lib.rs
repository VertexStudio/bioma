use derive_more::Deref;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod client;
pub mod prompts;
pub mod resources;
pub mod sampling;
pub mod schema;
pub mod server;
pub mod tools;
pub mod transport;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Deref)]
pub struct ConnectionId(Uuid);

impl ConnectionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl std::fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub enum JsonRpcMessage {
    Response(jsonrpc_core::Response),
    Request(jsonrpc_core::Request),
}

impl From<jsonrpc_core::Request> for JsonRpcMessage {
    fn from(request: jsonrpc_core::Request) -> Self {
        JsonRpcMessage::Request(request)
    }
}

impl From<jsonrpc_core::Response> for JsonRpcMessage {
    fn from(response: jsonrpc_core::Response) -> Self {
        JsonRpcMessage::Response(response)
    }
}

impl From<jsonrpc_core::MethodCall> for JsonRpcMessage {
    fn from(method_call: jsonrpc_core::MethodCall) -> Self {
        JsonRpcMessage::Request(jsonrpc_core::Request::Single(jsonrpc_core::Call::MethodCall(method_call)))
    }
}

impl From<jsonrpc_core::Notification> for JsonRpcMessage {
    fn from(notification: jsonrpc_core::Notification) -> Self {
        JsonRpcMessage::Request(jsonrpc_core::Request::Single(jsonrpc_core::Call::Notification(notification)))
    }
}
