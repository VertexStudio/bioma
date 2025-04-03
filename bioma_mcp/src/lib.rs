use derive_more::Deref;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod client;
pub mod logging;
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
struct RequestId {
    id_type: RequestIdType,
    num_value: Option<u64>,
    str_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum RequestIdType {
    Num,
    Str,
}

impl TryFrom<&jsonrpc_core::Id> for RequestId {
    type Error = anyhow::Error;

    fn try_from(id: &jsonrpc_core::Id) -> Result<Self, Self::Error> {
        match id {
            jsonrpc_core::Id::Num(n) => Ok(Self { id_type: RequestIdType::Num, num_value: Some(*n), str_value: None }),
            jsonrpc_core::Id::Str(s) => {
                Ok(Self { id_type: RequestIdType::Str, num_value: None, str_value: Some(s.clone()) })
            }
            jsonrpc_core::Id::Null => Err(anyhow::anyhow!("Null request IDs are not supported by MCP")),
        }
    }
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.id_type {
            RequestIdType::Num => write!(f, "{}", self.num_value.unwrap_or(0)),
            RequestIdType::Str => write!(f, "\"{}\"", self.str_value.clone().unwrap_or_default()),
        }
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
