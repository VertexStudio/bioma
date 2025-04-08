use derive_more::Deref;
use schema::JsonrpcrequestParams;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod client;
pub mod logging;
pub mod operation;
pub mod prompts;
pub mod resources;
pub mod schema;
pub mod server;
pub mod tools;
pub mod transport;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Deref)]
pub struct ConnectionId(String);

impl ConnectionId {
    pub fn new(prefix: Option<String>) -> Self {
        let connection_id = match prefix {
            Some(name) => format!("{}-{}", name, Uuid::new_v4()),
            None => Uuid::new_v4().to_string(),
        };

        Self(connection_id)
    }
}

impl std::fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MessageId {
    Num(u64),
    Str(String),
}

impl TryFrom<&jsonrpc_core::Id> for MessageId {
    type Error = anyhow::Error;

    fn try_from(id: &jsonrpc_core::Id) -> Result<Self, Self::Error> {
        match id {
            jsonrpc_core::Id::Num(n) => Ok(Self::Num(*n)),
            jsonrpc_core::Id::Str(s) => Ok(Self::Str(s.clone())),
            jsonrpc_core::Id::Null => Err(anyhow::anyhow!("Null request IDs are not supported by MCP")),
        }
    }
}

impl std::fmt::Display for MessageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageId::Num(n) => write!(f, "{}", n),
            MessageId::Str(s) => write!(f, "\"{}\"", s),
        }
    }
}

pub type RequestId = (ConnectionId, MessageId);

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Params {
    #[serde(flatten)]
    params: serde_json::Value,
    #[serde(flatten)]
    rpc_params: Option<JsonrpcrequestParams>,
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
