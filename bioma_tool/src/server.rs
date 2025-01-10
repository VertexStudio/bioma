use crate::prompts::PromptGetHandler;
use crate::resources::ResourceReadHandler;
use crate::schema::{
    CallToolRequestParams, CancelledNotificationParams, GetPromptRequestParams, Implementation,
    InitializeRequestParams, InitializeResult, ListPromptsResult, ListResourcesResult, ListToolsResult,
    ReadResourceRequestParams, ServerCapabilities,
};
use crate::tools::ToolCallHandler;
use crate::transport::{Transport, TransportType};
use anyhow::{Context, Result};
use jsonrpc_core::{MetaIoHandler, Metadata, Params};
use tokio::sync::mpsc;
use tracing::{debug, error, info};

#[derive(Default, Clone)]
struct ServerMetadata;
impl Metadata for ServerMetadata {}

pub trait ModelContextProtocolServer: Send + Sync + 'static {
    fn new() -> Self;
    fn get_capabilities(&self) -> ServerCapabilities;
    fn get_resources(&self) -> &Vec<Box<dyn ResourceReadHandler>>;
    fn get_prompts(&self) -> &Vec<Box<dyn PromptGetHandler>>;
    fn get_tools(&self) -> &Vec<Box<dyn ToolCallHandler>>;
}

pub async fn start<T: ModelContextProtocolServer>(name: &str, mut transport: TransportType) -> Result<()> {
    debug!("Starting ModelContextProtocol server: {}", name);

    let server = T::new();
    let mut io_handler = MetaIoHandler::default();

    let server = std::sync::Arc::new(server);
    let server_tools = server.clone();
    let server_resources = server.clone();
    let server_prompts = server.clone();
    let server_call = server.clone();
    let server_get_prompt = server.clone();
    let server_read_resource = server.clone();

    io_handler.add_method_with_meta("initialize", move |params: Params, _meta: ServerMetadata| {
        let server = server.clone();
        debug!("Handling initialize request");

        async move {
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

    io_handler.add_method("ping", move |_params| {
        debug!("Handling ping request");

        async move {
            info!("Successfully handled ping request");
            Ok(serde_json::json!({}))
        }
    });

    io_handler.add_method("resources/list", move |_params| {
        let server = server_resources.clone();
        debug!("Handling resources/list request");

        let resources = server.get_resources().iter().map(|resource| resource.def()).collect::<Vec<_>>();

        async move {
            let response = ListResourcesResult { next_cursor: None, resources: resources, meta: None };

            info!("Successfully handled resources/list request");
            Ok(serde_json::to_value(response).unwrap_or_default())
        }
    });

    io_handler.add_method("resources/read", move |params: Params| {
        let server = server_read_resource.clone();
        debug!("Handling resources/read request");

        async move {
            let params: ReadResourceRequestParams = params.parse().map_err(|e| {
                error!("Failed to parse resource read parameters: {}", e);
                jsonrpc_core::Error::invalid_params(e.to_string())
            })?;

            let resources = server.get_resources();
            let resource = resources.iter().find(|resource| resource.def().uri == params.uri);

            match resource {
                Some(resource) => {
                    let result = resource.read_boxed(params.uri.clone()).await.map_err(|e| {
                        error!("Resource read failed: {}", e);
                        jsonrpc_core::Error::internal_error()
                    })?;

                    info!("Successfully handled resources/read request for: {}", params.uri);
                    Ok(serde_json::to_value(result).unwrap_or_default())
                }
                None => {
                    error!("Unknown resource requested: {}", params.uri);
                    Err(jsonrpc_core::Error::method_not_found())
                }
            }
        }
    });

    io_handler.add_method("prompts/list", move |_params| {
        let server = server_prompts.clone();
        debug!("Handling prompts/list request");

        let prompts = server.get_prompts().iter().map(|prompt| prompt.def()).collect::<Vec<_>>();

        async move {
            let response = ListPromptsResult { next_cursor: None, prompts: prompts, meta: None };

            info!("Successfully handled prompts/list request");
            Ok(serde_json::to_value(response).unwrap_or_default())
        }
    });

    io_handler.add_method("prompts/get", move |params: Params| {
        let server = server_get_prompt.clone();
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
    });

    io_handler.add_method("tools/list", move |_params| {
        let server = server_tools.clone();
        debug!("Handling tools/list request");

        let tools = server.get_tools().iter().map(|tool| tool.def()).collect::<Vec<_>>();

        async move {
            let response = ListToolsResult { next_cursor: None, tools: tools, meta: None };

            info!("Successfully handled tools/list request");
            Ok(serde_json::to_value(response).unwrap_or_default())
        }
    });

    io_handler.add_method("tools/call", move |params: Params| {
        let server = server_call.clone();
        debug!("Handling tools/call request");

        async move {
            let params: CallToolRequestParams = params.parse().map_err(|e| {
                error!("Failed to parse tool call parameters: {}", e);
                jsonrpc_core::Error::invalid_params(e.to_string())
            })?;

            // Find the requested tool
            let tools = server.get_tools();
            let tool = tools.iter().find(|t| t.def().name == params.name);

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
    });

    let (tx, mut rx) = mpsc::channel(32);

    // Spawn the transport reader
    let mut transport_reader = transport.clone();
    tokio::spawn(async move {
        if let Err(e) = transport_reader.start(tx).await {
            error!("Transport error: {}", e);
        }
    });

    // Handle incoming messages
    while let Some(request) = rx.recv().await {
        let response = io_handler.handle_request(&request, ServerMetadata::default()).await.unwrap_or_else(|| {
            if !request.contains(r#""method":"notifications/"#) && !request.contains(r#""method":"cancelled"#) {
                error!("Error handling request");
                return r#"{"jsonrpc": "2.0", "error": {"code": -32603, "message": "Internal error"}, "id": null}"#
                    .to_string();
            }
            String::new()
        });

        if !response.is_empty() {
            if let Err(e) = transport.send(response).await {
                error!("Failed to send response: {}", e);
                return Err(e).context("Failed to send response");
            }
        }
    }

    Ok(())
}
