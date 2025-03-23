use crate::prompts::PromptGetHandler;
use crate::resources::ResourceReadHandler;
use crate::schema::{
    CallToolRequestParams, CancelledNotificationParams, GetPromptRequestParams, Implementation,
    InitializeRequestParams, InitializeResult, InitializedNotificationParams, ListPromptsRequestParams,
    ListPromptsResult, ListResourceTemplatesRequestParams, ListResourceTemplatesResult, ListResourcesRequestParams,
    ListResourcesResult, ListToolsRequestParams, ListToolsResult, PingRequestParams, ReadResourceRequestParams,
    ResourceUpdatedNotificationParams, ServerCapabilities, SubscribeRequestParams, UnsubscribeRequestParams,
};
use crate::tools::ToolCallHandler;
use crate::transport::sse::SseTransport;
use crate::transport::ws::WsTransport;
use crate::transport::{stdio::StdioTransport, Message, Transport, TransportType};
use crate::{ClientId, JsonRpcMessage};
use anyhow::{Context, Result};
use jsonrpc_core::{MetaIoHandler, Metadata, Params};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

/// Metadata associated with client requests, used for routing responses
/// and tracking per-client state.
#[derive(Clone)]
pub struct ServerMetadata {
    /// Unique identifier for the client connection
    pub client_id: ClientId,
}

impl Metadata for ServerMetadata {}

pub trait ModelContextProtocolServer: Send + Sync + 'static {
    fn get_transport_config(&self) -> &TransportConfig;
    fn get_capabilities(&self) -> &ServerCapabilities;
    fn get_resources(&self) -> &Vec<Box<dyn ResourceReadHandler>>;
    fn get_prompts(&self) -> &Vec<Box<dyn PromptGetHandler>>;
    fn create_tools(&self) -> Vec<Box<dyn ToolCallHandler>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StdioConfig {}

#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct SseConfig {
    #[builder(default = default_server_url())]
    pub endpoint: String,
    #[builder(default = default_channel_capacity())]
    pub channel_capacity: usize,
}

fn default_server_url() -> String {
    "127.0.0.1:8090".to_string()
}

fn default_channel_capacity() -> usize {
    32
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
    "127.0.0.1:9090".to_string()
}

impl Default for WsConfig {
    fn default() -> Self {
        Self::builder().build()
    }
}

#[derive(Debug, Clone)]
pub enum TransportConfig {
    Stdio(StdioConfig),
    Sse(SseConfig),
    Ws(WsConfig),
}

pub struct Session {
    pub client_id: ClientId,
    pub tools: Vec<Box<dyn ToolCallHandler>>,
}

pub struct Server<T: ModelContextProtocolServer> {
    server: Arc<RwLock<T>>,
    sessions: Arc<RwLock<HashMap<ClientId, Session>>>,
}

impl<T: ModelContextProtocolServer> Server<T> {
    pub fn new(server: T) -> Self {
        Server { sessions: Arc::new(RwLock::new(HashMap::new())), server: Arc::new(RwLock::new(server)) }
    }

    pub async fn start(&self) -> Result<()> {
        let transport_config = self.server.read().await.get_transport_config().clone();

        let (transport, mut on_message_rx, _on_error_rx, _on_close_rx) = match &transport_config {
            TransportConfig::Stdio(_config) => {
                let (on_message_tx, on_message_rx) = mpsc::channel::<Message>(32);
                let (on_error_tx, on_error_rx) = mpsc::channel(32);
                let (on_close_tx, on_close_rx) = mpsc::channel(32);

                let transport =
                    StdioTransport::new_server(on_message_tx.clone(), on_error_tx.clone(), on_close_tx.clone());
                (TransportType::Stdio(transport), on_message_rx, on_error_rx, on_close_rx)
            }
            TransportConfig::Sse(config) => {
                let (on_message_tx, on_message_rx) = mpsc::channel::<Message>(config.channel_capacity);
                let (on_error_tx, on_error_rx) = mpsc::channel(32);
                let (on_close_tx, on_close_rx) = mpsc::channel(32);

                let transport = SseTransport::new_server(
                    config.clone(),
                    on_message_tx.clone(),
                    on_error_tx.clone(),
                    on_close_tx.clone(),
                );
                (TransportType::Sse(transport), on_message_rx, on_error_rx, on_close_rx)
            }
            TransportConfig::Ws(config) => {
                let (on_message_tx, on_message_rx) = mpsc::channel::<Message>(32);
                let (on_error_tx, on_error_rx) = mpsc::channel(32);
                let (on_close_tx, on_close_rx) = mpsc::channel(32);

                let transport = WsTransport::new_server(
                    config.clone(),
                    on_message_tx.clone(),
                    on_error_tx.clone(),
                    on_close_tx.clone(),
                );
                (TransportType::Ws(transport), on_message_rx, on_error_rx, on_close_rx)
            }
        };

        let mut io_handler = MetaIoHandler::default();

        io_handler.add_method_with_meta("initialize", {
            let server = self.server.clone();
            let sessions = self.sessions.clone();

            move |params: Params, meta: ServerMetadata| {
                let server = server.clone();
                let sessions = sessions.clone();

                debug!("Handling initialize request");

                async move {
                    let capabilities = server.read().await.get_capabilities().clone();

                    let init_params: InitializeRequestParams = params.parse().map_err(|e| {
                        error!("Failed to parse initialize parameters: {}", e);
                        jsonrpc_core::Error::invalid_params(e.to_string())
                    })?;

                    let result = InitializeResult {
                        capabilities,
                        protocol_version: init_params.protocol_version,
                        server_info: Implementation {
                            name: "bioma-mcp-server".to_string(),
                            version: "0.1.0".to_string(),
                        },
                        instructions: Some("Bioma MCP server".to_string()),
                        meta: None,
                    };

                    let client_id = meta.client_id;

                    let server = server.read().await;
                    let tools = server.create_tools();

                    let session = Session { client_id: client_id.clone(), tools };
                    sessions.write().await.insert(client_id, session);

                    info!("Successfully handled initialize request");
                    Ok(serde_json::to_value(result).map_err(|e| {
                        error!("Failed to serialize initialize result: {}", e);
                        jsonrpc_core::Error::invalid_params(e.to_string())
                    })?)
                }
            }
        });

        io_handler.add_notification_with_meta("notifications/initialized", |params: Params, _meta: ServerMetadata| {
            match params.parse::<InitializedNotificationParams>() {
                Ok(_params) => {
                    info!("Received initialized notification");
                }
                Err(e) => {
                    error!("Failed to parse initialized notification params: {}", e);
                }
            }
        });

        io_handler.add_notification_with_meta("cancelled", move |params: Params, _meta: ServerMetadata| {
            match params.parse::<CancelledNotificationParams>() {
                Ok(cancel_params) => {
                    info!(
                        "Received cancellation for request {}: {}",
                        cancel_params.request_id,
                        cancel_params.reason.unwrap_or_default()
                    );
                }
                Err(e) => {
                    error!("Failed to parse cancellation params: {}", e);
                }
            }
        });

        io_handler.add_method_with_meta("ping", move |params: Params, _meta: ServerMetadata| {
            debug!("Handling ping request");

            async move {
                let _params: PingRequestParams = match params.parse() {
                    Ok(params) => params,
                    Err(e) => {
                        error!("Failed to parse ping parameters: {}", e);
                        return Err(jsonrpc_core::Error::invalid_params(e.to_string()));
                    }
                };

                info!("Successfully handled ping request");
                Ok(serde_json::json!({
                    "result": "pong",
                }))
            }
        });

        io_handler.add_method_with_meta("resources/list", {
            let server = self.server.clone();
            move |params: Params, _meta: ServerMetadata| {
                let server = server.clone();
                debug!("Handling resources/list request");

                async move {
                    let params: ListResourcesRequestParams = match params.parse() {
                        Ok(params) => params,
                        Err(e) => {
                            error!("Failed to parse resources/list parameters: {}", e);
                            return Err(jsonrpc_core::Error::invalid_params(e.to_string()));
                        }
                    };

                    // Here you could use params.cursor for pagination if needed
                    debug!("Resources list request with cursor: {:?}", params.cursor);

                    let resources =
                        server.read().await.get_resources().iter().map(|resource| resource.def()).collect::<Vec<_>>();

                    let response = ListResourcesResult { next_cursor: None, resources, meta: None };

                    info!("Successfully handled resources/list request");
                    Ok(serde_json::to_value(response).unwrap_or_default())
                }
            }
        });

        io_handler.add_method_with_meta("resources/read", {
            let server = self.server.clone();
            move |params: Params, _meta: ServerMetadata| {
                let server = server.clone();
                async move {
                    debug!("Handling resources/read request");

                    let params: ReadResourceRequestParams = match params.parse() {
                        Ok(params) => params,
                        Err(e) => {
                            error!("Failed to parse resources/read parameters: {}", e);
                            return Err(jsonrpc_core::Error::invalid_params(e.to_string()));
                        }
                    };

                    debug!("Requested resource URI: {}", params.uri);

                    let server = server.read().await;
                    let resources = server.get_resources();
                    let resource = resources.iter().find(|resource| resource.supports(&params.uri));

                    match resource {
                        Some(resource) => match resource.read_boxed(params.uri.clone()).await {
                            Ok(result) => {
                                info!("Successfully handled resources/read request for: {}", params.uri);
                                Ok(serde_json::to_value(result).map_err(|e| {
                                    error!("Failed to serialize resources/read result: {}", e);
                                    jsonrpc_core::Error::internal_error()
                                })?)
                            }
                            Err(e) => {
                                error!("Failed to read resource: {}", e);
                                Err(jsonrpc_core::Error::internal_error())
                            }
                        },
                        None => {
                            error!("Resource not found: {}", params.uri);
                            Err(jsonrpc_core::Error::invalid_params(format!("Resource not found: {}", params.uri)))
                        }
                    }
                }
            }
        });

        io_handler.add_method_with_meta("resources/templates/list", {
            let server = self.server.clone();
            move |params: Params, _meta: ServerMetadata| {
                let server = server.clone();
                async move {
                    debug!("Handling resources/templates/list request");

                    let params: ListResourceTemplatesRequestParams = match params.parse() {
                        Ok(params) => params,
                        Err(e) => {
                            error!("Failed to parse resources/templates/list parameters: {}", e);
                            return Err(jsonrpc_core::Error::invalid_params(e.to_string()));
                        }
                    };

                    // Here you could use params.cursor for pagination if needed
                    debug!("Resource templates list request with cursor: {:?}", params.cursor);

                    let server = server.read().await;
                    let resources = server.get_resources();

                    // Collect all resource templates
                    let resource_templates =
                        resources.iter().filter_map(|resource| resource.template()).collect::<Vec<_>>();

                    let response = ListResourceTemplatesResult { next_cursor: None, resource_templates, meta: None };

                    info!(
                        "Successfully handled resources/templates/list request, found {} templates",
                        response.resource_templates.len()
                    );
                    Ok(serde_json::to_value(response).unwrap_or_default())
                }
            }
        });

        io_handler.add_method_with_meta("resources/subscribe", {
            let server = self.server.clone();
            let transport = transport.clone();

            move |params: Params, meta: ServerMetadata| {
                let server = server.clone();
                let client_id = meta.client_id.clone();
                let mut transport = transport.clone();

                async move {
                    debug!("Handling resources/subscribe request");

                    let params: SubscribeRequestParams = match params.parse() {
                        Ok(params) => params,
                        Err(e) => {
                            error!("Failed to parse resources/subscribe parameters: {}", e);
                            return Err(jsonrpc_core::Error::invalid_params(e.to_string()));
                        }
                    };

                    debug!("Subscribe resource URI: {}", params.uri);

                    let server = server.read().await;
                    let resources = server.get_resources();
                    let resource = resources
                        .iter()
                        .find(|resource| resource.supports(&params.uri) && resource.supports_subscription(&params.uri));
                    let on_resource_updated_client_id = client_id.clone();
                    let on_resource_updated_uri = params.uri.clone();

                    let (on_resource_updated_tx, mut on_resource_updated_rx) = mpsc::channel::<()>(1);
                    tokio::spawn({
                        async move {
                            let params = ResourceUpdatedNotificationParams { uri: on_resource_updated_uri.clone() };
                            let request = jsonrpc_core::Notification {
                                jsonrpc: Some(jsonrpc_core::Version::V2),
                                method: "notifications/resources/updated".to_string(),
                                params: Params::Map(
                                    serde_json::to_value(params)
                                        .unwrap_or_default()
                                        .as_object()
                                        .cloned()
                                        .unwrap_or_default(),
                                ),
                            };
                            let message: JsonRpcMessage = request.into();
                            while let Some(_) = on_resource_updated_rx.recv().await {
                                if let Err(e) =
                                    transport.send(message.clone(), on_resource_updated_client_id.clone()).await
                                {
                                    error!("Failed to send error notification: {}", e);
                                }
                            }
                        }
                    });

                    match resource {
                        Some(resource) => match resource.subscribe(params.uri.clone(), on_resource_updated_tx).await {
                            Ok(_) => {
                                info!("Successfully subscribed to resource: {} for client: {}", params.uri, client_id);
                                Ok(serde_json::json!({}))
                            }
                            Err(e) => {
                                error!("Failed to subscribe to resource: {}", e);
                                Err(jsonrpc_core::Error::internal_error())
                            }
                        },
                        None => {
                            error!("Resource not found or doesn't support subscription: {}", params.uri);
                            Err(jsonrpc_core::Error::invalid_params(format!(
                                "Resource not found or doesn't support subscription: {}",
                                params.uri
                            )))
                        }
                    }
                }
            }
        });

        io_handler.add_method_with_meta("resources/unsubscribe", {
            let server = self.server.clone();
            move |params: Params, _meta: ServerMetadata| {
                let server = server.clone();
                async move {
                    debug!("Handling resources/unsubscribe request");

                    let params: UnsubscribeRequestParams = match params.parse() {
                        Ok(params) => params,
                        Err(e) => {
                            error!("Failed to parse resources/unsubscribe parameters: {}", e);
                            return Err(jsonrpc_core::Error::invalid_params(e.to_string()));
                        }
                    };

                    debug!("Unsubscribe resource URI: {}", params.uri);

                    let server = server.read().await;
                    let resources = server.get_resources();
                    let resource = resources
                        .iter()
                        .find(|resource| resource.supports(&params.uri) && resource.supports_subscription(&params.uri));

                    match resource {
                        Some(resource) => match resource.unsubscribe(params.uri.clone()).await {
                            Ok(_) => {
                                info!("Successfully unsubscribed from resource: {}", params.uri);
                                Ok(serde_json::json!({}))
                            }
                            Err(e) => {
                                error!("Failed to unsubscribe from resource: {}", e);
                                Err(jsonrpc_core::Error::internal_error())
                            }
                        },
                        None => {
                            error!("Resource not found or doesn't support subscription: {}", params.uri);
                            Err(jsonrpc_core::Error::invalid_params(format!(
                                "Resource not found or doesn't support subscription: {}",
                                params.uri
                            )))
                        }
                    }
                }
            }
        });

        io_handler.add_method_with_meta("prompts/list", {
            let server = self.server.clone();
            move |params: Params, _meta: ServerMetadata| {
                let server = server.clone();
                debug!("Handling prompts/list request");

                async move {
                    let params: ListPromptsRequestParams = match params.parse() {
                        Ok(params) => params,
                        Err(e) => {
                            error!("Failed to parse prompts/list parameters: {}", e);
                            return Err(jsonrpc_core::Error::invalid_params(e.to_string()));
                        }
                    };

                    // Here you could use params.cursor for pagination if needed
                    debug!("Prompts list request with cursor: {:?}", params.cursor);

                    let server = server.read().await;
                    let prompts = server.get_prompts().iter().map(|prompt| prompt.def()).collect::<Vec<_>>();

                    let response = ListPromptsResult { next_cursor: None, prompts, meta: None };

                    info!("Successfully handled prompts/list request");
                    Ok(serde_json::to_value(response).unwrap_or_default())
                }
            }
        });

        io_handler.add_method_with_meta("prompts/get", {
            let server = self.server.clone();
            move |params: Params, _meta: ServerMetadata| {
                let server = server.clone();
                debug!("Handling prompts/get request");

                async move {
                    let params: GetPromptRequestParams = params.parse().map_err(|e| {
                        error!("Failed to parse prompt get parameters: {}", e);
                        jsonrpc_core::Error::invalid_params(e.to_string())
                    })?;

                    // Find the requested prompt
                    let server = server.read().await;
                    let prompts = server.get_prompts();
                    let prompt = prompts.iter().find(|p| p.def().name == params.name);

                    match prompt {
                        Some(prompt) => {
                            let result = prompt.get_boxed(params.arguments).await.map_err(|e| {
                                error!("Prompt execution failed: {}", e);
                                jsonrpc_core::Error::internal_error()
                            })?;

                            info!("Successfully handled prompts/get request for: {}", params.name);
                            Ok(serde_json::to_value(result).map_err(|e| {
                                error!("Failed to serialize prompt result: {}", e);
                                jsonrpc_core::Error::invalid_params(e.to_string())
                            })?)
                        }
                        None => {
                            error!("Unknown prompt requested: {}", params.name);
                            Err(jsonrpc_core::Error::method_not_found())
                        }
                    }
                }
            }
        });

        io_handler.add_method_with_meta("tools/list", {
            let sessions = self.sessions.clone();

            move |params: Params, meta: ServerMetadata| {
                let sessions = sessions.clone();

                debug!("Handling tools/list request");

                async move {
                    let params: ListToolsRequestParams = match params.parse() {
                        Ok(params) => params,
                        Err(e) => {
                            error!("Failed to parse tools/list parameters: {}", e);
                            return Err(jsonrpc_core::Error::invalid_params(e.to_string()));
                        }
                    };

                    // Here you could use params.cursor for pagination if needed
                    debug!("Tools list request with cursor: {:?}", params.cursor);

                    let client_id = meta.client_id.clone();
                    let sessions = sessions.read().await;

                    let tools = if let Some(session) = sessions.get(&client_id) {
                        session.tools.iter().map(|tool| tool.def()).collect::<Vec<_>>()
                    } else {
                        vec![]
                    };
                    let response = ListToolsResult { next_cursor: None, tools, meta: None };
                    info!("Successfully handled tools/list request");
                    Ok(serde_json::to_value(response).unwrap_or_default())
                }
            }
        });

        io_handler.add_method_with_meta("tools/call", {
            let sessions = self.sessions.clone();

            move |params: Params, meta: ServerMetadata| {
                let sessions = sessions.clone();

                debug!("Handling tools/call request");

                async move {
                    let params: CallToolRequestParams = params.parse().map_err(|e| {
                        error!("Failed to parse tool call parameters: {}", e);
                        jsonrpc_core::Error::invalid_params(e.to_string())
                    })?;

                    let sessions = sessions.read().await;

                    let tool = if let Some(session) = sessions.get(&meta.client_id) {
                        session.tools.iter().find(|t| t.def().name == params.name)
                    } else {
                        None
                    };

                    match tool {
                        Some(tool) => {
                            let result = tool.call_boxed(params.arguments).await.map_err(|e| {
                                error!("Tool execution failed: {}", e);
                                jsonrpc_core::Error::internal_error()
                            })?;

                            info!("Successfully handled tool call for: {}", params.name);
                            Ok(serde_json::to_value(result).map_err(|e| {
                                error!("Failed to serialize tool call result: {}", e);
                                jsonrpc_core::Error::invalid_params(e.to_string())
                            })?)
                        }
                        None => {
                            error!("Unknown tool requested: {}", params.name);
                            Err(jsonrpc_core::Error::method_not_found())
                        }
                    }
                }
            }
        });

        // Start the transport
        let mut transport = transport.clone();
        if let Err(e) = transport.start().await {
            error!("Transport error: {}", e);
            return Err(e).context("Failed to start transport");
        }

        // Handle messages based on transport type
        match transport {
            TransportType::Stdio(_) => {
                while let Some(stdio_request) = on_message_rx.recv().await {
                    match &stdio_request.message {
                        JsonRpcMessage::Request(request) => match request {
                            jsonrpc_core::Request::Single(jsonrpc_core::Call::MethodCall(_call)) => {
                                let Some(response) = io_handler
                                    .handle_rpc_request(
                                        request.clone(),
                                        ServerMetadata { client_id: stdio_request.client_id.clone() },
                                    )
                                    .await
                                else {
                                    continue;
                                };

                                if let Err(e) = transport.send(response.into(), stdio_request.client_id.clone()).await {
                                    error!("Failed to send response: {}", e);
                                    return Err(e).context("Failed to send response");
                                }
                            }
                            jsonrpc_core::Request::Single(jsonrpc_core::Call::Notification(notification)) => {
                                debug!("Handled notification: {:?}", notification.method);
                            }
                            _ => {
                                warn!("Unsupported request: {:?}", request);
                            }
                        },
                        JsonRpcMessage::Response(response) => {
                            error!("Received response: {:?}", response);
                        }
                    }
                }
            }
            TransportType::Sse(_) => {
                while let Some(sse_message) = on_message_rx.recv().await {
                    match &sse_message.message {
                        JsonRpcMessage::Request(request) => match request {
                            jsonrpc_core::Request::Single(jsonrpc_core::Call::MethodCall(_call)) => {
                                let metadata = ServerMetadata { client_id: sse_message.client_id.clone() };

                                let Some(response) = io_handler.handle_rpc_request(request.clone(), metadata).await
                                else {
                                    continue;
                                };

                                if let Err(e) = transport.send(response.into(), sse_message.client_id.clone()).await {
                                    error!("Failed to send SSE response: {}", e);
                                    return Err(e).context("Failed to send SSE response");
                                }
                            }
                            jsonrpc_core::Request::Single(jsonrpc_core::Call::Notification(notification)) => {
                                debug!("Handled notification: {:?}", notification.method);
                            }
                            _ => {
                                warn!("Unsupported request: {:?}", request);
                            }
                        },
                        _ => {}
                    }
                }
            }
            TransportType::Ws(_) => {
                while let Some(ws_message) = on_message_rx.recv().await {
                    match &ws_message.message {
                        JsonRpcMessage::Request(request) => match request {
                            jsonrpc_core::Request::Single(jsonrpc_core::Call::MethodCall(_call)) => {
                                let metadata = ServerMetadata { client_id: ws_message.client_id.clone() };

                                let Some(response) = io_handler.handle_rpc_request(request.clone(), metadata).await
                                else {
                                    continue;
                                };

                                if let Err(e) = transport.send(response.into(), ws_message.client_id.clone()).await {
                                    error!("Failed to send WebSocket response: {}", e);
                                    return Err(e).context("Failed to send WebSocket response");
                                }
                            }
                            jsonrpc_core::Request::Single(jsonrpc_core::Call::Notification(notification)) => {
                                debug!("Handled notification: {:?}", notification.method);
                            }
                            _ => {
                                warn!("Unsupported request: {:?}", request);
                            }
                        },
                        _ => {}
                    }
                }
            }
        }

        Ok(())
    }
}
