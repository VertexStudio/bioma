use bioma_mcp::{
    schema::{CallToolResult, TextContent},
    server::RequestContext,
    tools::{ToolDef, ToolError},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct HealthArgs {
    #[schemars(description = "Check specific service (all, database, chat, embeddings)")]
    service: Option<String>,
}

#[derive(Clone)]
pub struct Health;

impl Health {
    pub fn new() -> Self {
        Self
    }
}

impl ToolDef for Health {
    const NAME: &'static str = "health";
    const DESCRIPTION: &'static str = "Check health of the RAG services";
    type Args = HealthArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        let service = args.service.unwrap_or_else(|| "all".to_string());

        // In a real implementation, this would perform actual health checks
        // For now, we'll just return mock health status

        let health_status = match service.to_lowercase().as_str() {
            "database" => serde_json::json!({
                "service": "database",
                "status": "healthy",
                "latency_ms": 12,
                "details": {
                    "connection_pool": "active",
                    "version": "5.1.3"
                }
            }),
            "chat" => serde_json::json!({
                "service": "chat",
                "status": "healthy",
                "latency_ms": 35,
                "details": {
                    "model": "online",
                    "cache_hit_ratio": 0.78
                }
            }),
            "embeddings" => serde_json::json!({
                "service": "embeddings",
                "status": "healthy",
                "latency_ms": 28,
                "details": {
                    "model": "online",
                    "dimensions": 1024
                }
            }),
            "all" => serde_json::json!({
                "overall": "healthy",
                "services": [
                    {
                        "service": "database",
                        "status": "healthy",
                        "latency_ms": 12
                    },
                    {
                        "service": "chat",
                        "status": "healthy",
                        "latency_ms": 35
                    },
                    {
                        "service": "embeddings",
                        "status": "healthy",
                        "latency_ms": 28
                    },
                    {
                        "service": "indexer",
                        "status": "healthy",
                        "latency_ms": 15
                    },
                    {
                        "service": "retriever",
                        "status": "healthy",
                        "latency_ms": 42
                    }
                ]
            }),
            _ => return Err(ToolError::Custom(format!("Unknown service: {}", service))),
        };

        let response =
            serde_json::to_string_pretty(&health_status).unwrap_or_else(|_| format!("Error serializing health status"));

        Ok(CallToolResult {
            content: vec![
                serde_json::to_value(TextContent { type_: "text".to_string(), text: response, annotations: None })
                    .map_err(ToolError::ResultSerialize)?,
            ],
            is_error: Some(false),
            meta: None,
        })
    }
}
