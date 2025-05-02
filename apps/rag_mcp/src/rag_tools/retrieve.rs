use bioma_mcp::{
    schema::{CallToolResult, TextContent},
    server::{Context, RequestContext},
    tools::{ToolDef, ToolError},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct RetrieveArgs {
    #[schemars(description = "Query text to retrieve relevant context for")]
    query: String,

    #[schemars(description = "Maximum number of results to return")]
    limit: Option<usize>,

    #[schemars(description = "Filter results by source name")]
    source_name: Option<String>,

    #[schemars(description = "Similarity threshold (0.0-1.0)")]
    threshold: Option<f32>,

    #[schemars(description = "Output format (text or json)")]
    format: Option<String>,
}

#[derive(Clone)]
pub struct Retrieve {
    context: Context,
}

impl Retrieve {
    pub fn new(context: Context) -> Self {
        Self { context }
    }
}

impl ToolDef for Retrieve {
    const NAME: &'static str = "retrieve";
    const DESCRIPTION: &'static str = "Retrieves context from indexed sources based on semantic similarity";
    type Args = RetrieveArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        // In a real implementation, this would connect to the retriever actor
        // For now, we'll just return mock results

        let limit = args.limit.unwrap_or(3);
        let source_filter = args.source_name.clone().unwrap_or_else(|| "all sources".to_string());

        let mock_results = vec![
            format!("Relevant context 1 for query: '{}' from {}", args.query, source_filter),
            format!("Relevant context 2 for query: '{}' from {}", args.query, source_filter),
            format!("Relevant context 3 for query: '{}' from {}", args.query, source_filter),
        ];

        let results = mock_results.into_iter().take(limit).collect::<Vec<_>>();

        let format = args.format.unwrap_or_else(|| "text".to_string());
        let response = if format == "json" {
            serde_json::to_string(&results).unwrap_or_else(|_| results.join("\n"))
        } else {
            results.join("\n\n")
        };

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
