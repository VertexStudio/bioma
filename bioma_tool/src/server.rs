use crate::prompts::PromptGetHandler;
use crate::resources::ResourceReadHandler;
use crate::schema::{
    CallToolRequestParams, CancelledNotificationParams, GetPromptRequestParams, Implementation,
    InitializeRequestParams, InitializeResult, ListPromptsResult, ListResourcesResult, ListToolsResult,
    ReadResourceRequestParams, ServerCapabilities,
};
use crate::tools::ToolCallHandler;
use crate::transport::sse::{SseMessage, SseMetadata, SseTransport};
use crate::transport::ws::{WsMessage, WsMetadata, WsTransport};
use crate::transport::{stdio::StdioTransport, Transport, TransportType};
use crate::{ClientId, JsonRpcMessage};
use anyhow::{Context, Result};
use jsonrpc_core::{MetaIoHandler, Metadata, Params};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
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

pub async fn start<T: ModelContextProtocolServer>(name: &str, transport: TransportConfig) -> Result<()> {
    let server = T::new();
    start_with_impl(name, transport, server).await
}

pub async fn start_with_impl<T: ModelContextProtocolServer>(
    _name: &str,
    transport: TransportConfig,
    server: T,
) -> Result<()> {
    let server = Arc::new(server);

    // Create a message handler for the server
    let mut io_handler = MetaIoHandler::default();

    // Store active tool call handlers per client
    let tools = Arc::new(RwLock::new(HashMap::new()));

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

    io_handler.add_notification_with_meta("notifications/initialized", |_params, _meta: ServerMetadata| {
        info!("Received initialized notification");
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

    io_handler.add_method_with_meta("ping", move |_params, _meta: ServerMetadata| {
        debug!("Handling ping request");

        async move {
            info!("Successfully handled ping request");
            Ok(serde_json::json!({}))
        }
    });

    io_handler.add_method_with_meta("resources/list", {
        let server = server.clone();
        move |_params, _meta: ServerMetadata| {
            let server = server.clone();
            debug!("Handling resources/list request");

            let resources = server.get_resources().iter().map(|resource| resource.def()).collect::<Vec<_>>();

            async move {
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
                    Some(resource) => match resource.subscribe(params.uri.clone()).await {
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
                    Some(resource) => match resource.unsubscribe(params.uri.clone()).await {
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
        move |_params, _meta: ServerMetadata| {
            let server = server.clone();
            debug!("Handling prompts/list request");

            let prompts = server.get_prompts().iter().map(|prompt| prompt.def()).collect::<Vec<_>>();

            async move {
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
        move |_params, meta: ServerMetadata| {
            debug!("Handling tools/list request");
            let client_tools = client_tools.clone();

            async move {
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

    // Add support for resource templates
    io_handler.add_method_with_meta("resources/templates/list", {
        let server = server.clone();
        move |_params: Params, _meta: ServerMetadata| {
            let server = server.clone();
            async move {
                debug!("Handling resources/templates/list request");

                let resources = server.get_resources();

                // Collect all resource templates
                let templates = resources.iter().filter_map(|resource| resource.template()).collect::<Vec<_>>();

                // Create the response
                let response = serde_json::json!({
                    "nextCursor": null,
                    "resourceTemplates": templates
                });

                info!("Successfully handled resources/templates/list request, found {} templates", templates.len());
                Ok(response)
            }
        }
    });

    match transport {
        TransportConfig::Stdio(_config) => {
            let (on_message_tx, mut on_message_rx) = mpsc::channel::<JsonRpcMessage>(32);
            let (on_error_tx, _on_error_rx) = mpsc::channel(32);
            let (on_close_tx, _on_close_rx) = mpsc::channel(32);

            let transport = StdioTransport::new_server(on_message_tx, on_error_tx, on_close_tx);
            let mut transport_type = TransportType::Stdio(transport);

            if let Err(e) = transport_type.start().await {
                error!("Transport error: {}", e);
                return Err(e).context("Failed to start transport");
            }

            // Generate a fixed client ID for stdio transport
            let client_id = ClientId::new();

            while let Some(request) = on_message_rx.recv().await {
                match &request {
                    JsonRpcMessage::Request(request) => match request {
                        jsonrpc_core::Request::Single(jsonrpc_core::Call::MethodCall(_call)) => {
                            let Some(response) = io_handler
                                .handle_rpc_request(request.clone(), ServerMetadata { client_id: client_id.clone() })
                                .await
                            else {
                                continue;
                            };

                            if let Err(e) = transport_type.send(response.into(), serde_json::Value::Null).await {
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
        TransportConfig::Sse(config) => {
            let (on_message_tx, mut on_message_rx) = mpsc::channel::<SseMessage>(32);
            let (on_error_tx, _on_error_rx) = mpsc::channel(32);
            let (on_close_tx, _on_close_rx) = mpsc::channel(32);

            let transport = SseTransport::new_server(config, on_message_tx, on_error_tx, on_close_tx);
            let mut transport_type = TransportType::Sse(transport);

            if let Err(e) = transport_type.start().await {
                error!("Transport error: {}", e);
                return Err(e).context("Failed to start transport");
            }

            while let Some(sse_message) = on_message_rx.recv().await {
                match &sse_message.message {
                    JsonRpcMessage::Request(request) => match request {
                        jsonrpc_core::Request::Single(jsonrpc_core::Call::MethodCall(_call)) => {
                            let metadata = ServerMetadata { client_id: sse_message.client_id.clone() };

                            let Some(response) = io_handler.handle_rpc_request(request.clone(), metadata).await else {
                                continue;
                            };

                            let sse_metadata = SseMetadata { client_id: sse_message.client_id.clone() };

                            if let Err(e) = transport_type
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
        TransportConfig::Ws(config) => {
            let (on_message_tx, mut on_message_rx) = mpsc::channel::<WsMessage>(32);
            let (on_error_tx, _on_error_rx) = mpsc::channel(32);
            let (on_close_tx, _on_close_rx) = mpsc::channel(32);

            let transport = WsTransport::new_server(config, on_message_tx, on_error_tx, on_close_tx);
            let mut transport_type = TransportType::Ws(transport);

            if let Err(e) = transport_type.start().await {
                error!("Transport error: {}", e);
                return Err(e).context("Failed to start transport");
            }

            while let Some(ws_message) = on_message_rx.recv().await {
                match &ws_message.message {
                    JsonRpcMessage::Request(request) => match request {
                        jsonrpc_core::Request::Single(jsonrpc_core::Call::MethodCall(_call)) => {
                            let metadata = ServerMetadata { client_id: ws_message.client_id.clone() };

                            let Some(response) = io_handler.handle_rpc_request(request.clone(), metadata).await else {
                                continue;
                            };

                            let ws_metadata = WsMetadata { client_id: ws_message.client_id.clone() };

                            if let Err(e) = transport_type
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
