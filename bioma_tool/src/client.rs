use crate::transport::Transport;
use anyhow::Result;
use jsonrpc_core::Params;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

use crate::schema::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, GetPromptRequestParams, GetPromptResult, Implementation,
    InitializeRequestParams, InitializeResult, ListPromptsRequestParams, ListPromptsResult, ListResourcesRequestParams,
    ListResourcesResult, ListToolsRequestParams, ListToolsResult, ReadResourceRequestParams, ReadResourceResult,
    ServerCapabilities,
};
use crate::transport::TransportType;
use crate::utils::ping::{ConnectionHealth, PingConfig};

pub struct ServerConfig {
    pub command: String,
    pub args: Vec<String>,
}

pub struct Client {
    transport: TransportType,
    pub server_capabilities: Arc<RwLock<Option<ServerCapabilities>>>,
    request_counter: Arc<RwLock<u64>>,
    response_rx: mpsc::Receiver<String>,
    health: Arc<ConnectionHealth>,
}

impl Client {
    pub async fn new(transport: TransportType, ping_config: Option<PingConfig>) -> Result<Self> {
        let (tx, rx) = mpsc::channel::<String>(1);
        let (reconnect_tx, mut reconnect_rx) = mpsc::channel::<()>(1);

        // Initialize health monitoring
        let health = ConnectionHealth::new(ping_config.unwrap_or_default());
        let transport_clone = transport.clone();
        health.start_monitor(transport_clone, reconnect_tx).await?;

        // Handle reconnection signals
        let transport_for_reconnect = transport.clone();
        tokio::spawn(async move {
            while let Some(()) = reconnect_rx.recv().await {
                debug!("Attempting reconnection...");
                if let Err(e) = Self::handle_reconnection(&transport_for_reconnect).await {
                    error!("Reconnection failed: {}", e);
                }
            }
        });

        // Start transport once during initialization
        let mut transport_clone = transport.clone();
        tokio::spawn(async move {
            if let Err(e) = transport_clone.start(tx).await {
                error!("Transport error: {}", e);
            }
        });

        Ok(Self {
            transport,
            server_capabilities: Arc::new(RwLock::new(None)),
            request_counter: Arc::new(RwLock::new(0)),
            response_rx: rx,
            health: Arc::new(health),
        })
    }

    // Add reconnection handler
    async fn handle_reconnection(transport: &TransportType) -> Result<()> {
        let mut retry_count = 0;
        let max_retries = 3;
        let base_delay_ms = 1000; // Start with 1 second delay

        while retry_count < max_retries {
            debug!("Reconnection attempt {} of {}", retry_count + 1, max_retries);

            // Create a new request channel
            let (tx, _rx) = mpsc::channel::<String>(32);

            // Attempt to restart transport
            if let Err(e) = transport.clone().start(tx).await {
                warn!("Transport restart failed: {}", e);
                // Exponential backoff with jitter
                let jitter = rand::random::<u64>() % 500;
                let delay = base_delay_ms * 2u64.pow(retry_count) + jitter;
                tokio::time::sleep(Duration::from_millis(delay)).await;
                retry_count += 1;
                continue;
            }

            // Re-initialize the connection
            if let Ok(()) = Self::reinitialize_connection(transport).await {
                info!("Successfully reconnected and reinitialized");
                return Ok(());
            }

            retry_count += 1;
        }

        Err(anyhow::anyhow!("Failed to reconnect after {} attempts", max_retries))
    }

    async fn reinitialize_connection(transport: &TransportType) -> Result<()> {
        let init_request = InitializeRequestParams {
            protocol_version: "2024-11-05".to_string(),
            capabilities: ClientCapabilities::default(),
            client_info: Implementation {
                name: "bioma-mcp-client".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };

        let request = jsonrpc_core::MethodCall {
            jsonrpc: Some(jsonrpc_core::Version::V2),
            method: "initialize".to_string(),
            params: Params::Map(serde_json::to_value(init_request)?.as_object().unwrap().clone()),
            id: jsonrpc_core::Id::Num(1),
        };

        transport.clone().send(serde_json::to_string(&request)?).await?;

        // Send initialized notification
        let notification = jsonrpc_core::Notification {
            jsonrpc: Some(jsonrpc_core::Version::V2),
            method: "notifications/initialized".to_string(),
            params: Params::Map(serde_json::Map::new()),
        };

        transport.clone().send(serde_json::to_string(&notification)?).await?;

        Ok(())
    }

    pub async fn initialize(&mut self, client_info: Implementation) -> Result<InitializeResult> {
        let params = InitializeRequestParams {
            protocol_version: "2024-11-05".to_string(),
            capabilities: ClientCapabilities::default(),
            client_info,
        };
        let response = self.request("initialize".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn list_resources(&mut self, params: Option<ListResourcesRequestParams>) -> Result<ListResourcesResult> {
        debug!("Sending resources/list request");
        let response = self.request("resources/list".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn read_resource(&mut self, params: ReadResourceRequestParams) -> Result<ReadResourceResult> {
        debug!("Sending resources/read request");
        let response = self.request("resources/read".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn list_prompts(&mut self, params: Option<ListPromptsRequestParams>) -> Result<ListPromptsResult> {
        debug!("Sending prompts/list request");
        let response = self.request("prompts/list".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn get_prompt(&mut self, params: GetPromptRequestParams) -> Result<GetPromptResult> {
        debug!("Sending prompts/get request");
        let response = self.request("prompts/get".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn list_tools(&mut self, params: Option<ListToolsRequestParams>) -> Result<ListToolsResult> {
        debug!("Sending tools/list request");
        let response = self.request("tools/list".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn call_tool(&mut self, params: CallToolRequestParams) -> Result<CallToolResult> {
        debug!("Sending tools/call request");
        let response = self.request("tools/call".to_string(), serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(response)?)
    }

    pub async fn request(&mut self, method: String, params: serde_json::Value) -> Result<serde_json::Value> {
        let mut counter = self.request_counter.write().await;
        *counter += 1;
        let id = *counter;

        // Create proper JSON-RPC 2.0 request
        let request = jsonrpc_core::MethodCall {
            jsonrpc: Some(jsonrpc_core::Version::V2),
            method: method.clone(),
            params: Params::Map(params.as_object().map(|obj| obj.clone()).unwrap_or_default()),
            id: jsonrpc_core::Id::Num(id),
        };

        // Send request
        let request_str = serde_json::to_string(&request)?;
        self.transport.send(request_str).await?;

        // Parse response as proper JSON-RPC response
        if let Some(response) = self.response_rx.recv().await {
            // Handle pong responses
            if method == "ping" {
                self.health.handle_pong().await;
                return Ok(serde_json::json!({}));
            }

            let response: jsonrpc_core::Response = serde_json::from_str(&response)?;
            match response {
                jsonrpc_core::Response::Single(output) => match output {
                    jsonrpc_core::Output::Success(success) => Ok(success.result),
                    jsonrpc_core::Output::Failure(failure) => {
                        error!("RPC error: {:?}", failure.error);
                        anyhow::bail!("RPC error: {:?}", failure.error)
                    }
                },
                _ => anyhow::bail!("Unexpected response type"),
            }
        } else {
            anyhow::bail!("No response received")
        }
    }
}
