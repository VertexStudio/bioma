use crate::prompts::PromptGetHandler;
use crate::resources::{NotificationCallback, ResourceError, ResourceManager, ResourceReadHandler};
use crate::schema::{
    CallToolRequestParams, CancelledNotificationParams, GetPromptRequestParams, Implementation,
    InitializeRequestParams, InitializeResult, InitializedNotificationParams, ListPromptsRequestParams,
    ListPromptsResult, ListResourceTemplatesRequestParams, ListResourceTemplatesResult, ListResourcesRequestParams,
    ListResourcesResult, ListToolsRequestParams, ListToolsResult, PingRequestParams, ReadResourceRequestParams,
    ResourceUpdatedNotificationParams, ServerCapabilities,
};
use crate::tools::ToolCallHandler;
use crate::transport::sse::{SseMessage, SseMetadata, SseTransport};
use crate::transport::ws::{WsMessage, WsMetadata, WsTransport};
use crate::transport::{stdio::StdioTransport, Transport, TransportType};
use crate::{ClientId, JsonRpcMessage};
use anyhow::{Context, Result};
use jsonrpc_core::{MetaIoHandler, Metadata, Params};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{debug, error, info};

#[derive(Clone)]
pub struct ServerMetadata {
    pub client_id: ClientId,
}

impl Metadata for ServerMetadata {}

pub trait ModelContextProtocolServer: Send + Sync + 'static {
    fn new() -> Self;
    fn get_capabilities(&self) -> ServerCapabilities;
    fn get_resources(&self) -> &Vec<Box<dyn ResourceReadHandler>>;
    fn get_prompts(&self) -> &Vec<Box<dyn PromptGetHandler>>;
    fn create_tools(&self) -> Vec<Box<dyn ToolCallHandler>>;
}

pub struct StdioConfig {}

/// Server configuration with builder pattern
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

pub enum TransportConfig {
    Stdio(StdioConfig),
    Sse(SseConfig),
    Ws(WsConfig),
}

/// Helper struct for notification management
#[derive(Clone)]
struct NotificationManager {
    transport: Arc<Mutex<Option<TransportType>>>,
}

impl NotificationManager {
    fn new() -> Self {
        Self { transport: Arc::new(Mutex::new(None)) }
    }

    fn set_transport(&self, transport: TransportType) {
        let mut t = self.transport.blocking_lock();
        *t = Some(transport);
    }

    async fn send_notification(&self, client_id: ClientId, method: &str, params: impl Serialize) -> Result<()> {
        let transport_guard = self.transport.lock().await;
        if let Some(transport) = &*transport_guard {
            // Create the notification
            let params_value = serde_json::to_value(params).map_err(|e| {
                error!("Failed to serialize notification params: {}", e);
                anyhow::anyhow!("Failed to serialize notification params: {}", e)
            })?;

            let notification = jsonrpc_core::Notification {
                jsonrpc: Some(jsonrpc_core::Version::V2),
                method: method.to_string(),
                params: jsonrpc_core::Params::Map(serde_json::from_value(params_value).map_err(|e| {
                    error!("Failed to convert params to Map: {}", e);
                    anyhow::anyhow!("Failed to convert params to Map: {}", e)
                })?),
            };

            // Create metadata based on the client_id
            let meta = match transport {
                TransportType::Stdio(_) => serde_json::Value::Null,
                TransportType::Sse(_) => {
                    let sse_metadata = SseMetadata { client_id: client_id.clone() };
                    serde_json::to_value(&sse_metadata).unwrap_or_default()
                }
                TransportType::Ws(_) => {
                    let ws_metadata = WsMetadata { client_id: client_id.clone() };
                    serde_json::to_value(&ws_metadata).unwrap_or_default()
                }
            };

            // Clone the transport since we can't mutably borrow it
            let mut transport_clone = transport.clone();

            // Send the notification
            transport_clone
                .send(
                    JsonRpcMessage::Request(jsonrpc_core::Request::Single(jsonrpc_core::Call::Notification(
                        notification,
                    ))),
                    meta,
                )
                .await
                .map_err(|e| {
                    error!("Failed to send notification: {}", e);
                    anyhow::anyhow!("Failed to send notification: {}", e)
                })
        } else {
            error!("Cannot send notification: transport not set");
            Err(anyhow::anyhow!("Transport not set"))
        }
    }
}

/// Helper function to get the ResourceManager from a resource
fn get_resource_manager(resource: &dyn ResourceReadHandler) -> Option<Arc<ResourceManager>> {
    // Try to get the resource manager from the FileSystem resource
    if let Some(fs) = resource.as_any().downcast_ref::<crate::resources::filesystem::FileSystem>() {
        return Some(fs.get_resource_manager());
    }

    // Add other resource types as needed
    None
}

pub async fn start<T: ModelContextProtocolServer>(name: &str, transport: TransportConfig) -> Result<()> {
    let server = T::new();
    start_with_impl(name, transport, server).await
}

pub async fn start_with_impl<T: ModelContextProtocolServer>(
    _name: &str,
    transport_config: TransportConfig,
    server: T,
) -> Result<()> {
    let server = Arc::new(server);

    // Create a message handler for the server
    let mut io_handler = MetaIoHandler::default();

    // Store active tool call handlers per client
    let tools = Arc::new(RwLock::new(HashMap::new()));

    // Create notification manager
    let notification_manager = Arc::new(NotificationManager::new());

    // Create and configure the transport based on the config
    let (transport_type, message_rx) = match transport_config {
        TransportConfig::Stdio(_config) => {
            let (on_message_tx, on_message_rx) = mpsc::channel::<JsonRpcMessage>(32);
            let (on_error_tx, _on_error_rx) = mpsc::channel(32);
            let (on_close_tx, _on_close_rx) = mpsc::channel(32);

            let transport = StdioTransport::new_server(on_message_tx, on_error_tx, on_close_tx);
            (TransportType::Stdio(transport), Box::new(on_message_rx) as Box<dyn Any>)
        }
        TransportConfig::Sse(config) => {
            let (on_message_tx, on_message_rx) = mpsc::channel::<SseMessage>(config.channel_capacity);
            let (on_error_tx, _on_error_rx) = mpsc::channel(32);
            let (on_close_tx, _on_close_rx) = mpsc::channel(32);

            let transport = SseTransport::new_server(config, on_message_tx, on_error_tx, on_close_tx);
            (TransportType::Sse(transport), Box::new(on_message_rx) as Box<dyn Any>)
        }
        TransportConfig::Ws(config) => {
            let (on_message_tx, on_message_rx) = mpsc::channel::<WsMessage>(32);
            let (on_error_tx, _on_error_rx) = mpsc::channel(32);
            let (on_close_tx, _on_close_rx) = mpsc::channel(32);

            let transport = WsTransport::new_server(config, on_message_tx, on_error_tx, on_close_tx);
            (TransportType::Ws(transport), Box::new(on_message_rx) as Box<dyn Any>)
        }
    };

    // Set up the notification manager with the transport
    notification_manager.set_transport(transport_type.clone());

    // Setup resources and their notification callbacks
    for resource in server.get_resources() {
        // Get the resource manager if available
        if let Some(resource_manager) = get_resource_manager(resource.as_ref()) {
            // Create a notification callback
            let notification_mgr = notification_manager.clone();
            let callback: NotificationCallback =
                Box::new(move |client_id, params: ResourceUpdatedNotificationParams| {
                    let nm = notification_mgr.clone();
                    Box::pin(async move {
                        match nm.send_notification(client_id, "notifications/resources/updated", params).await {
                            Ok(_) => Ok(()),
                            Err(e) => Err(ResourceError::Custom(format!("Failed to send notification: {}", e))),
                        }
                    })
                });

            // Set the callback
            if let Err(e) = resource_manager.set_notification_callback(callback) {
                error!("Failed to set notification callback: {}", e);
            }
        }
    }

    // Create tools per client
    io_handler.add_method_with_meta("initialize", {
        let server = server.clone();
        let client_tools = tools.clone();

        move |params: Params, meta: ServerMetadata| {
            let client_tools = client_tools.clone();
            let server = server.clone();
            debug!("Handling initialize request");

            async move {
                let tools = server.create_tools();
                client_tools.write().await.insert(meta.client_id, tools);

                let init_params: InitializeRequestParams = params.parse().map_err(|e| {
                    error!("Failed to parse initialize parameters: {}", e);
                    jsonrpc_core::Error::invalid_params(e.to_string())
                })?;

                let result = InitializeResult {
                    capabilities: server.get_capabilities(),
                    protocol_version: init_params.protocol_version,
                    server_info: Implementation { name: "bioma-mcp-server".to_string(), version: "0.1.0".to_string() },
                    instructions: Some("Bioma MCP server".to_string()),
                    meta: None,
                };

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
            Ok(serde_json::json!({}))
        }
    });

    io_handler.add_method_with_meta("resources/list", {
        let server = server.clone();
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

                let resources = server.get_resources().iter().map(|resource| resource.def()).collect::<Vec<_>>();

                let response = ListResourcesResult { next_cursor: None, resources, meta: None };

                info!("Successfully handled resources/list request");
                Ok(serde_json::to_value(response).unwrap_or_default())
            }
        }
    });

    io_handler.add_method_with_meta("resources/read", {
        let server = server.clone();
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
        let server = server.clone();
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

    // Add support for resource subscriptions
    io_handler.add_method_with_meta("resources/subscribe", {
        let server = server.clone();
        move |params: Params, meta: ServerMetadata| {
            let server = server.clone();
            let client_id = meta.client_id.clone();
            async move {
                debug!("Handling resources/subscribe request");

                #[derive(Deserialize)]
                struct SubscribeParams {
                    uri: String,
                }

                let params: SubscribeParams = match params.parse() {
                    Ok(params) => params,
                    Err(e) => {
                        error!("Failed to parse resources/subscribe parameters: {}", e);
                        return Err(jsonrpc_core::Error::invalid_params(e.to_string()));
                    }
                };

                debug!("Subscribe resource URI: {}", params.uri);

                let resources = server.get_resources();
                let resource = resources
                    .iter()
                    .find(|resource| resource.supports(&params.uri) && resource.supports_subscription(&params.uri));

                match resource {
                    Some(resource) => match resource.subscribe(params.uri.clone(), client_id.clone()).await {
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
        let server = server.clone();
        move |params: Params, meta: ServerMetadata| {
            let server = server.clone();
            let client_id = meta.client_id.clone();
            async move {
                debug!("Handling resources/unsubscribe request");

                #[derive(Deserialize)]
                struct UnsubscribeParams {
                    uri: String,
                }

                let params: UnsubscribeParams = match params.parse() {
                    Ok(params) => params,
                    Err(e) => {
                        error!("Failed to parse resources/unsubscribe parameters: {}", e);
                        return Err(jsonrpc_core::Error::invalid_params(e.to_string()));
                    }
                };

                debug!("Unsubscribe resource URI: {}", params.uri);

                let resources = server.get_resources();
                let resource = resources
                    .iter()
                    .find(|resource| resource.supports(&params.uri) && resource.supports_subscription(&params.uri));

                match resource {
                    Some(resource) => match resource.unsubscribe(params.uri.clone(), client_id.clone()).await {
                        Ok(_) => {
                            info!("Successfully unsubscribed from resource: {} for client: {}", params.uri, client_id);
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
        let server = server.clone();
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

                let prompts = server.get_prompts().iter().map(|prompt| prompt.def()).collect::<Vec<_>>();

                let response = ListPromptsResult { next_cursor: None, prompts, meta: None };

                info!("Successfully handled prompts/list request");
                Ok(serde_json::to_value(response).unwrap_or_default())
            }
        }
    });

    io_handler.add_method_with_meta("prompts/get", {
        let server = server.clone();
        move |params: Params, _meta: ServerMetadata| {
            let server = server.clone();
            debug!("Handling prompts/get request");

            async move {
                let params: GetPromptRequestParams = params.parse().map_err(|e| {
                    error!("Failed to parse prompt get parameters: {}", e);
                    jsonrpc_core::Error::invalid_params(e.to_string())
                })?;

                // Find the requested prompt
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
        let client_tools = tools.clone();
        move |params: Params, meta: ServerMetadata| {
            debug!("Handling tools/list request");
            let client_tools = client_tools.clone();

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

                let tools = if let Some(tools) = client_tools.read().await.get(&meta.client_id) {
                    tools.iter().map(|tool| tool.def()).collect::<Vec<_>>()
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
        let client_tools = tools.clone();
        move |params: Params, meta: ServerMetadata| {
            debug!("Handling tools/call request");
            let client_tools = client_tools.clone();

            async move {
                let params: CallToolRequestParams = params.parse().map_err(|e| {
                    error!("Failed to parse tool call parameters: {}", e);
                    jsonrpc_core::Error::invalid_params(e.to_string())
                })?;

                let tool = client_tools.read().await;
                let tool = if let Some(tools) = tool.get(&meta.client_id) {
                    tools.iter().find(|t| t.def().name == params.name)
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
    let mut transport = transport_type.clone();
    if let Err(e) = transport.start().await {
        error!("Transport error: {}", e);
        return Err(e).context("Failed to start transport");
    }

    // Handle messages based on transport type
    match transport_type {
        TransportType::Stdio(_) => {
            let mut rx = message_rx
                .downcast::<mpsc::Receiver<JsonRpcMessage>>()
                .expect("Failed to downcast to JsonRpcMessage receiver");

            // Generate a fixed client ID for stdio transport
            let client_id = ClientId::new();

            while let Some(request) = rx.recv().await {
                match &request {
                    JsonRpcMessage::Request(request) => match request {
                        jsonrpc_core::Request::Single(jsonrpc_core::Call::MethodCall(_call)) => {
                            let Some(response) = io_handler
                                .handle_rpc_request(request.clone(), ServerMetadata { client_id: client_id.clone() })
                                .await
                            else {
                                continue;
                            };

                            if let Err(e) = transport.send(response.into(), serde_json::Value::Null).await {
                                error!("Failed to send response: {}", e);
                                return Err(e).context("Failed to send response");
                            }
                        }
                        jsonrpc_core::Request::Single(jsonrpc_core::Call::Notification(_notification)) => {}
                        _ => {}
                    },
                    JsonRpcMessage::Response(_) => {}
                }
            }
        }
        TransportType::Sse(_) => {
            let mut rx =
                message_rx.downcast::<mpsc::Receiver<SseMessage>>().expect("Failed to downcast to SseMessage receiver");

            while let Some(sse_message) = rx.recv().await {
                match &sse_message.message {
                    JsonRpcMessage::Request(request) => match request {
                        jsonrpc_core::Request::Single(jsonrpc_core::Call::MethodCall(_call)) => {
                            let metadata = ServerMetadata { client_id: sse_message.client_id.clone() };

                            let Some(response) = io_handler.handle_rpc_request(request.clone(), metadata).await else {
                                continue;
                            };

                            let sse_metadata = SseMetadata { client_id: sse_message.client_id.clone() };

                            if let Err(e) = transport
                                .send(response.into(), serde_json::to_value(&sse_metadata).unwrap_or_default())
                                .await
                            {
                                error!("Failed to send SSE response: {}", e);
                                return Err(e).context("Failed to send SSE response");
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
        TransportType::Ws(_) => {
            let mut rx =
                message_rx.downcast::<mpsc::Receiver<WsMessage>>().expect("Failed to downcast to WsMessage receiver");

            while let Some(ws_message) = rx.recv().await {
                match &ws_message.message {
                    JsonRpcMessage::Request(request) => match request {
                        jsonrpc_core::Request::Single(jsonrpc_core::Call::MethodCall(_call)) => {
                            let metadata = ServerMetadata { client_id: ws_message.client_id.clone() };

                            let Some(response) = io_handler.handle_rpc_request(request.clone(), metadata).await else {
                                continue;
                            };

                            let ws_metadata = WsMetadata { client_id: ws_message.client_id.clone() };

                            if let Err(e) = transport
                                .send(response.into(), serde_json::to_value(&ws_metadata).unwrap_or_default())
                                .await
                            {
                                error!("Failed to send WebSocket response: {}", e);
                                return Err(e).context("Failed to send WebSocket response");
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
    }

    Ok(())
}
