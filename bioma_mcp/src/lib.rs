use derive_more::Deref;
use schema::ProgressNotificationParams;
use schema::ProgressToken;
use schema::RequestParamsMeta;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use uuid::Uuid;

pub mod client;
pub mod logging;
pub mod operation;
pub mod progress;
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

#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct RequestParams<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<RequestParamsMeta>,

    #[serde(flatten)]
    pub params: T,
}

#[derive(Clone)]
pub struct OutgoingRequest<E> {
    counter: Arc<RwLock<u64>>,
    conn_id: ConnectionId,
    pending_requests: Arc<Mutex<HashMap<RequestId, PendingRequest<E>>>>,
    progress_trackers: Arc<Mutex<HashMap<ProgressToken, mpsc::Sender<ProgressNotificationParams>>>>,
}

struct PendingRequest<E> {
    response_sender: oneshot::Sender<Result<serde_json::Value, E>>,
    progress_token: Option<ProgressToken>,
}

impl<E> OutgoingRequest<E>
where
    E: Send + 'static,
{
    pub fn new(conn_id: ConnectionId) -> Self {
        Self {
            counter: Arc::new(RwLock::new(0)),
            conn_id,
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            progress_trackers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn create_request(
        &self,
        track_progress: bool,
    ) -> (
        RequestId,
        jsonrpc_core::Id,
        oneshot::Receiver<Result<serde_json::Value, E>>,
        Option<mpsc::Receiver<ProgressNotificationParams>>,
    ) {
        let id = {
            let mut counter = self.counter.write().await;
            *counter += 1;
            *counter
        };

        let jsonrpc_id = jsonrpc_core::Id::Num(id);
        let message_id = MessageId::Num(id);
        let request_id = (self.conn_id.clone(), message_id);

        let (response_tx, response_rx) = oneshot::channel();

        let (progress_token, progress_receiver) = if track_progress {
            let token = ProgressToken::from(uuid::Uuid::new_v4().to_string());
            let (tx, rx) = mpsc::channel::<ProgressNotificationParams>(32);

            let mut progress_trackers = self.progress_trackers.lock().await;
            progress_trackers.insert(token.clone(), tx);

            (Some(token), Some(rx))
        } else {
            (None, None)
        };

        let pending_request = PendingRequest { response_sender: response_tx, progress_token };

        let mut pending_requests = self.pending_requests.lock().await;
        pending_requests.insert(request_id.clone(), pending_request);

        (request_id, jsonrpc_id, response_rx, progress_receiver)
    }

    pub async fn create_subrequest(
        &self,
        progress: Option<(ProgressToken, mpsc::Sender<ProgressNotificationParams>)>,
    ) -> (RequestId, jsonrpc_core::Id, oneshot::Receiver<Result<serde_json::Value, E>>) {
        let id = {
            let mut counter = self.counter.write().await;
            *counter += 1;
            *counter
        };

        let jsonrpc_id = jsonrpc_core::Id::Num(id);
        let message_id = MessageId::Num(id);
        let request_id = (self.conn_id.clone(), message_id);

        let (response_tx, response_rx) = oneshot::channel();

        let progress_token = if let Some((token, sender)) = progress {
            let mut progress_trackers = self.progress_trackers.lock().await;
            progress_trackers.insert(token.clone(), sender);
            Some(token)
        } else {
            None
        };

        let pending_request = PendingRequest { response_sender: response_tx, progress_token };

        let mut pending_requests = self.pending_requests.lock().await;
        pending_requests.insert(request_id.clone(), pending_request);

        (request_id, jsonrpc_id, response_rx)
    }

    pub async fn complete_request(&self, request_id: &RequestId, result: Result<serde_json::Value, E>) -> bool {
        let request = {
            let mut pending_requests = self.pending_requests.lock().await;
            pending_requests.remove(request_id)
        };

        if let Some(request) = request {
            let _ = request.response_sender.send(result);

            if let Some(token) = request.progress_token {
                let mut progress_trackers = self.progress_trackers.lock().await;
                progress_trackers.remove(&token);
            }

            true
        } else {
            false
        }
    }

    pub async fn handle_progress(&self, params: ProgressNotificationParams) {
        let token = params.progress_token.clone();
        let progress_trackers = self.progress_trackers.lock().await;

        if let Some(sender) = progress_trackers.get(&token) {
            let _ = sender.send(params).await;
        } else {
        }
    }

    pub async fn request_exists(&self, request_id: &RequestId) -> bool {
        let pending_requests = self.pending_requests.lock().await;
        pending_requests.contains_key(request_id)
    }

    pub fn connection_id(&self) -> &ConnectionId {
        &self.conn_id
    }

    pub async fn prepare_params_with_token(
        &self,
        params: serde_json::Value,
        progress_token: Option<ProgressToken>,
    ) -> Result<serde_json::Value, serde_json::Error> {
        if let Some(token) = progress_token {
            let meta = crate::RequestParamsMeta { progress_token: Some(token) };
            let modified_params = crate::RequestParams { meta: Some(meta), params };
            serde_json::to_value(modified_params)
        } else {
            Ok(params)
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
