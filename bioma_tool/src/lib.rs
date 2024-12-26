use anyhow::{Context, Result};
use jsonrpc_core::{MetaIoHandler, Metadata, Params};
use tokio::sync::mpsc;
use tools::ToolCallHandler;
use tracing::{debug, error, info};
use transport::{Transport, TransportType};

pub mod schema;
pub mod tools;
pub mod transport;

use schema::{
    CallToolRequestParams, CancelledNotificationParams, Implementation, InitializeRequestParams,
    InitializeResult, ListPromptsResult, ListResourcesResult, ListToolsResult, Prompt, Resource,
    ServerCapabilities,
};

#[derive(Default, Clone)]
struct ServerMetadata;
impl Metadata for ServerMetadata {}

pub trait ModelContextProtocolServer: Send + Sync + 'static {
    fn new() -> Self;
    fn get_capabilities(&self) -> ServerCapabilities;
    fn get_resources(&self) -> &Vec<Resource>;
    fn get_prompts(&self) -> &Vec<Prompt>;
    fn get_tools(&self) -> &Vec<Box<dyn ToolCallHandler>>;
}

pub async fn start_server<T: ModelContextProtocolServer>(
    mut transport: TransportType,
) -> Result<()> {
    let server = T::new();
    let mut io_handler = MetaIoHandler::default();

    let server = std::sync::Arc::new(server);
    let server_tools = server.clone();
    let server_resources = server.clone();
    let server_prompts = server.clone();
    let server_call = server.clone();

    io_handler.add_method_with_meta(
        "initialize",
        move |params: Params, _meta: ServerMetadata| {
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
                    server_info: Implementation {
                        name: "rust-mcp-server".to_string(),
                        version: "0.1.0".to_string(),
                    },
                    instructions: Some("Basic MCP server with tool support".to_string()),
                    meta: None,
                };

                info!("Successfully handled initialize request");
                Ok(serde_json::to_value(result).map_err(|e| {
                    error!("Failed to serialize initialize result: {}", e);
                    jsonrpc_core::Error::invalid_params(e.to_string())
                })?)
            }
        },
    );

    io_handler.add_notification_with_meta(
        "notifications/initialized",
        |_params, _meta: ServerMetadata| {
            info!("Received initialized notification");
        },
    );

    io_handler.add_notification_with_meta(
        "cancelled",
        move |params: Params, _meta: ServerMetadata| match params
            .parse::<CancelledNotificationParams>()
        {
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
        },
    );

    io_handler.add_method("resources/list", move |_params| {
        let server = server_resources.clone();
        debug!("Handling resources/list request");

        async move {
            let response = ListResourcesResult {
                next_cursor: None,
                resources: server.get_resources().clone(),
                meta: None,
            };

            info!("Successfully handled resources/list request");
            Ok(serde_json::to_value(response).unwrap_or_default())
        }
    });

    io_handler.add_method("prompts/list", move |_params| {
        let server = server_prompts.clone();
        debug!("Handling prompts/list request");

        async move {
            let response = ListPromptsResult {
                next_cursor: None,
                prompts: server.get_prompts().clone(),
                meta: None,
            };

            info!("Successfully handled prompts/list request");
            Ok(serde_json::to_value(response).unwrap_or_default())
        }
    });

    io_handler.add_method("tools/list", move |_params| {
        let server = server_tools.clone();
        debug!("Handling tools/list request");

        let tools = server
            .get_tools()
            .iter()
            .map(|tool| tool.def())
            .collect::<Vec<_>>();

        async move {
            let response = ListToolsResult {
                next_cursor: None,
                tools: tools,
                meta: None,
            };

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
        let response = io_handler
            .handle_request(&request, ServerMetadata::default())
            .await
            .unwrap_or_else(|| {
                if !request.contains(r#""method":"notifications/"#) && 
                   !request.contains(r#""method":"cancelled"#) {
                    error!("Error handling request");
                    return r#"{"jsonrpc": "2.0", "error": {"code": -32603, "message": "Internal error"}, "id": null}"#.to_string();
                }
                String::new()
            });

        if !response.is_empty() {
            if let Err(e) = transport.send_response(response).await {
                error!("Failed to send response: {}", e);
                return Err(e).context("Failed to send response");
            }
        }
    }

    Ok(())
}
