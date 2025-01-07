use crate::schema::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, GetPromptRequestParams, GetPromptResult, Implementation,
    InitializeRequestParams, InitializeResult, ListPromptsRequestParams, ListPromptsResult, ListResourcesRequestParams,
    ListResourcesResult, ListToolsRequestParams, ListToolsResult, ReadResourceRequestParams, ReadResourceResult,
    ServerCapabilities,
};
use crate::transport::{stdio::StdioTransport, Transport, TransportType};
use anyhow::Result;
use jsonrpc_core::Params;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub name: String,
    #[serde(default = "default_transport")]
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
}

fn default_transport() -> String {
    "stdio".to_string()
}

pub struct ModelContextProtocolClient {
    transport: TransportType,
    pub server_capabilities: Arc<RwLock<Option<ServerCapabilities>>>,
    request_counter: Arc<RwLock<u64>>,
    response_rx: mpsc::Receiver<String>,
}

impl ModelContextProtocolClient {
    pub async fn new(server: ServerConfig) -> Result<Self> {
        let (tx, rx) = mpsc::channel::<String>(1);

        let transport = StdioTransport::new_client(&server)?;

        let transport = match server.transport.as_str() {
            "stdio" => TransportType::Stdio(transport),
            _ => return Err(anyhow::anyhow!("Invalid transport type")),
        };

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
        })
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
            method,
            params: Params::Map(params.as_object().map(|obj| obj.clone()).unwrap_or_default()),
            id: jsonrpc_core::Id::Num(id),
        };

        // Send request
        let request_str = serde_json::to_string(&request)?;
        self.transport.send(request_str).await?;

        // Parse response as proper JSON-RPC response
        if let Some(response) = self.response_rx.recv().await {
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
