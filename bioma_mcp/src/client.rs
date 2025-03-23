use crate::schema::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, GetPromptRequestParams, GetPromptResult, Implementation,
    InitializeRequestParams, InitializeResult, InitializedNotificationParams, ListPromptsRequestParams,
    ListPromptsResult, ListResourceTemplatesRequestParams, ListResourceTemplatesResult, ListResourcesRequestParams,
    ListResourcesResult, ListToolsRequestParams, ListToolsResult, ReadResourceRequestParams, ReadResourceResult, Root,
    RootsListChangedNotification, RootsListChangedNotificationParams, ServerCapabilities,
};
use crate::transport::sse::SseTransport;
use crate::transport::ws::WsTransport;
use crate::transport::{stdio::StdioTransport, Transport, TransportSender, TransportType};
use crate::{ConnectionId, JsonRpcMessage};
use anyhow::Error;
use jsonrpc_core::Params;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StdioConfig {
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct SseConfig {
    #[serde(default = "default_server_url")]
    #[builder(default = default_server_url())]
    pub endpoint: String,
}

fn default_server_url() -> String {
    "http://127.0.0.1:8090".to_string()
}

impl Default for SseConfig {
    fn default() -> Self {
        Self::builder().build()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct WsConfig {
    #[builder(default = default_ws_server_url())]
    pub endpoint: String,
}

fn default_ws_server_url() -> String {
    "ws://127.0.0.1:9090".to_string()
}

impl Default for WsConfig {
    fn default() -> Self {
        Self::builder().build()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "transport")]
pub enum TransportConfig {
    #[serde(rename = "stdio")]
    Stdio(StdioConfig),
    #[serde(rename = "sse")]
    Sse(SseConfig),
    #[serde(rename = "ws")]
    Ws(WsConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct ServerConfig {
    pub name: String,
    #[serde(flatten)]
    pub transport: TransportConfig,
    #[serde(default = "default_request_timeout")]
    #[builder(default = default_request_timeout())]
    pub request_timeout: u64,
}

fn default_request_timeout() -> u64 {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct ClientConfig {
    pub server: ServerConfig,
}

pub trait ModelContextProtocolClient: Send + Sync + 'static {
    fn get_server_config(&self) -> impl Future<Output = ServerConfig> + Send;
    fn get_capabilities(&self) -> impl Future<Output = ClientCapabilities> + Send;
    fn get_roots(&self) -> impl Future<Output = Vec<Root>> + Send;
}

type RequestId = u64;
type ResponseSender = oneshot::Sender<Result<serde_json::Value, ClientError>>;
type PendingRequests = Arc<Mutex<HashMap<RequestId, ResponseSender>>>;

pub struct Client<T: ModelContextProtocolClient> {
    client: T,
    transport: TransportType,
    transport_sender: TransportSender,
    server_capabilities: Arc<RwLock<Option<ServerCapabilities>>>,
    request_counter: Arc<RwLock<u64>>,
    start_handle: JoinHandle<Result<(), Error>>,
    #[allow(unused)]
    message_handler: JoinHandle<()>,
    pending_requests: PendingRequests,
    #[allow(unused)]
    on_error_rx: mpsc::Receiver<Error>,
    #[allow(unused)]
    on_close_rx: mpsc::Receiver<()>,
    conn_id: ConnectionId,
}

impl<T: ModelContextProtocolClient> Client<T> {
    pub async fn new(client: T) -> Result<Self, ClientError> {
        let (on_message_tx, mut on_message_rx) = mpsc::channel::<JsonRpcMessage>(1);
        let (on_error_tx, on_error_rx) = mpsc::channel::<Error>(1);
        let (on_close_tx, on_close_rx) = mpsc::channel::<()>(1);

        let server_config = client.get_server_config().await;

        let mut transport = match &server_config.transport {
            TransportConfig::Stdio(config) => {
                let transport = StdioTransport::new_client(&config, on_message_tx, on_error_tx, on_close_tx).await;
                let transport = match transport {
                    Ok(transport) => transport,
                    Err(e) => return Err(ClientError::Transport(format!("Client new: {}", e).into())),
                };
                TransportType::Stdio(transport)
            }
            TransportConfig::Sse(config) => {
                let transport = SseTransport::new_client(config, on_message_tx, on_error_tx, on_close_tx);
                let transport = match transport {
                    Ok(transport) => transport,
                    Err(e) => return Err(ClientError::Transport(format!("Client new: {}", e).into())),
                };
                TransportType::Sse(transport)
            }
            TransportConfig::Ws(config) => {
                let transport = WsTransport::new_client(config, on_message_tx, on_error_tx, on_close_tx);
                let transport = match transport {
                    Ok(transport) => transport,
                    Err(e) => return Err(ClientError::Transport(format!("Client new: {}", e).into())),
                };
                TransportType::Ws(transport)
            }
        };

        let transport_sender = transport.sender();

        let conn_id = ConnectionId::new();

        let start_handle =
            transport.start().await.map_err(|e| ClientError::Transport(format!("Start: {}", e).into()))?;

        let pending_requests = Arc::new(Mutex::new(HashMap::<u64, ResponseSender>::new()));
        let pending_requests_clone = pending_requests.clone();

        let message_handler = tokio::spawn({
            let pending_requests = pending_requests_clone;
            async move {
                while let Some(message) = on_message_rx.recv().await {
                    match message {
                        JsonRpcMessage::Response(jsonrpc_core::Response::Single(output)) => match output {
                            jsonrpc_core::Output::Success(success) => {
                                if let jsonrpc_core::Id::Num(id) = success.id {
                                    let mut requests = pending_requests.lock().await;
                                    if let Some(sender) = requests.remove(&id) {
                                        let _ = sender.send(Ok(success.result));
                                    }
                                }
                            }
                            jsonrpc_core::Output::Failure(failure) => {
                                if let jsonrpc_core::Id::Num(id) = failure.id {
                                    let mut requests = pending_requests.lock().await;
                                    if let Some(sender) = requests.remove(&id) {
                                        let _ = sender.send(Err(ClientError::Request(
                                            format!("RPC error: {:?}", failure.error).into(),
                                        )));
                                    }
                                }
                            }
                        },
                        JsonRpcMessage::Request(request) => match request {
                            jsonrpc_core::Request::Single(jsonrpc_core::Call::Notification(notification)) => {
                                info!("Got notification: {:?}", notification);
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }
        });

        Ok(Self {
            client,
            transport,
            transport_sender,
            server_capabilities: Arc::new(RwLock::new(None)),
            request_counter: Arc::new(RwLock::new(0)),
            start_handle,
            message_handler,
            pending_requests,
            on_error_rx,
            on_close_rx,
            conn_id,
        })
    }

    pub async fn initialize(&mut self, client_info: Implementation) -> Result<InitializeResult, ClientError> {
        let params = InitializeRequestParams {
            protocol_version: "2024-11-05".to_string(),
            capabilities: self.client.get_capabilities().await,
            client_info,
        };
        let response = self.request("initialize".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn initialized(&mut self) -> Result<(), ClientError> {
        let params = InitializedNotificationParams { meta: None };
        self.notify("notifications/initialized".to_string(), serde_json::to_value(params)?).await?;
        Ok(())
    }

    pub async fn list_resources(
        &mut self,
        params: Option<ListResourcesRequestParams>,
    ) -> Result<ListResourcesResult, ClientError> {
        debug!("Server {} - Sending resources/list request", self.client.get_server_config().await.name);
        let response = self.request("resources/list".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn read_resource(
        &mut self,
        params: ReadResourceRequestParams,
    ) -> Result<ReadResourceResult, ClientError> {
        debug!("Server {} - Sending resources/read request", self.client.get_server_config().await.name);
        let response = self.request("resources/read".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn list_resource_templates(
        &mut self,
        params: Option<ListResourceTemplatesRequestParams>,
    ) -> Result<ListResourceTemplatesResult, ClientError> {
        debug!("Server {} - Sending resources/templates/list request", self.client.get_server_config().await.name);
        let response = self.request("resources/templates/list".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn subscribe_resource(&mut self, uri: String) -> Result<(), ClientError> {
        debug!(
            "Server {} - Sending resources/subscribe request for {}",
            self.client.get_server_config().await.name,
            uri
        );
        let params = serde_json::json!({ "uri": uri });
        let _response = self.request("resources/subscribe".to_string(), params).await?;
        Ok(())
    }

    pub async fn unsubscribe_resource(&mut self, uri: String) -> Result<(), ClientError> {
        debug!(
            "Server {} - Sending resources/unsubscribe request for {}",
            self.client.get_server_config().await.name,
            uri
        );
        let params = serde_json::json!({ "uri": uri });
        let _response = self.request("resources/unsubscribe".to_string(), params).await?;
        Ok(())
    }

    pub async fn list_prompts(
        &mut self,
        params: Option<ListPromptsRequestParams>,
    ) -> Result<ListPromptsResult, ClientError> {
        debug!("Server {} - Sending prompts/list request", self.client.get_server_config().await.name);
        let response = self.request("prompts/list".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn get_prompt(&mut self, params: GetPromptRequestParams) -> Result<GetPromptResult, ClientError> {
        debug!("Server {} - Sending prompts/get request", self.client.get_server_config().await.name);
        let response = self.request("prompts/get".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn list_tools(&mut self, params: Option<ListToolsRequestParams>) -> Result<ListToolsResult, ClientError> {
        debug!("Server {} - Sending tools/list request", self.client.get_server_config().await.name);
        let response = self.request("tools/list".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn call_tool(&mut self, params: CallToolRequestParams) -> Result<CallToolResult, ClientError> {
        debug!("Server {} - Sending tools/call request", self.client.get_server_config().await.name);
        let response = self.request("tools/call".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    async fn request(&mut self, method: String, params: serde_json::Value) -> Result<serde_json::Value, ClientError> {
        let mut counter = self.request_counter.write().await;
        *counter += 1;
        let id = *counter;

        let request = jsonrpc_core::MethodCall {
            jsonrpc: Some(jsonrpc_core::Version::V2),
            method,
            params: Params::Map(params.as_object().cloned().unwrap_or_default()),
            id: jsonrpc_core::Id::Num(id),
        };

        let (response_tx, response_rx) = oneshot::channel();

        {
            let mut pending = self.pending_requests.lock().await;
            pending.insert(id, response_tx);
        }

        let conn_id = self.conn_id.clone();

        if let Err(e) = self.transport_sender.send(request.into(), conn_id).await {
            let mut pending = self.pending_requests.lock().await;
            pending.remove(&id);
            return Err(ClientError::Transport(format!("Send: {}", e).into()));
        }

        match tokio::time::timeout(
            std::time::Duration::from_secs(self.client.get_server_config().await.request_timeout),
            response_rx,
        )
        .await
        {
            Ok(response) => match response {
                Ok(result) => result,
                Err(_) => Err(ClientError::Request("Response channel closed".into())),
            },
            Err(_) => {
                let mut pending = self.pending_requests.lock().await;
                pending.remove(&id);
                Err(ClientError::Request("Request timed out".into()))
            }
        }
    }

    pub async fn notify(&mut self, method: String, params: serde_json::Value) -> Result<(), ClientError> {
        let notification = jsonrpc_core::Notification {
            jsonrpc: Some(jsonrpc_core::Version::V2),
            method,
            params: Params::Map(params.as_object().cloned().unwrap_or_default()),
        };

        let conn_id = self.conn_id.clone();

        self.transport_sender
            .send(notification.into(), conn_id)
            .await
            .map_err(|e| ClientError::Transport(format!("Send: {}", e).into()))
    }

    pub async fn close(&mut self) -> Result<(), ClientError> {
        self.transport.close().await.map_err(|e| ClientError::Transport(format!("Close: {}", e).into()))?;
        self.start_handle.abort();
        Ok(())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ClientError {
    #[error("Invalid transport type: {0}")]
    Transport(Cow<'static, str>),
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("Request: {0}")]
    Request(Cow<'static, str>),
}

impl<T: ModelContextProtocolClient> std::fmt::Debug for Client<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ModelContextProtocolClient")
    }
}
