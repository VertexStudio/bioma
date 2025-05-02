use bioma_mcp::{
    schema::{CallToolResult, TextContent},
    server::{Context, RequestContext},
    tools::{ToolDef, ToolError},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ListSourcesArgs {
    #[schemars(description = "Filter sources by name pattern")]
    name_filter: Option<String>,

    #[schemars(description = "Sort by field (created, name, size)")]
    sort_by: Option<String>,

    #[schemars(description = "Sort direction (asc, desc)")]
    sort_direction: Option<String>,

    #[schemars(description = "Maximum number of sources to return")]
    limit: Option<usize>,
}

#[derive(Clone)]
pub struct ListSources {
    context: Context,
}

impl ListSources {
    pub fn new(context: Context) -> Self {
        Self { context }
    }
}

impl ToolDef for ListSources {
    const NAME: &'static str = "list_sources";
    const DESCRIPTION: &'static str = "List indexed sources available for retrieval";
    type Args = ListSourcesArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        // In a real implementation, this would query the database for sources
        // For now, we'll just return mock sources

        let limit = args.limit.unwrap_or(10);
        let sort_by = args.sort_by.unwrap_or_else(|| "name".to_string());
        let sort_direction = args.sort_direction.unwrap_or_else(|| "asc".to_string());

        // Mock sources
        let mock_sources = vec![
            serde_json::json!({
                "id": "source-1",
                "name": "Documentation",
                "created_at": "2023-10-15T14:30:00Z",
                "chunks": 45,
                "size": 125_000,
            }),
            serde_json::json!({
                "id": "source-2",
                "name": "Research Papers",
                "created_at": "2023-11-20T09:15:00Z",
                "chunks": 78,
                "size": 320_000,
            }),
            serde_json::json!({
                "id": "source-3",
                "name": "User Guides",
                "created_at": "2023-12-05T16:45:00Z",
                "chunks": 32,
                "size": 95_000,
            }),
        ];

        // Filter by name if specified
        let filtered_sources = if let Some(filter) = args.name_filter {
            mock_sources.into_iter().filter(|s| s["name"].as_str().unwrap_or("").contains(&filter)).collect::<Vec<_>>()
        } else {
            mock_sources
        };

        // Take only up to the limit
        let sources = filtered_sources.into_iter().take(limit).collect::<Vec<_>>();

        let result = serde_json::json!({
            "sources": sources,
            "total": sources.len(),
            "sort_by": sort_by,
            "sort_direction": sort_direction,
        });

        let response =
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| format!("Error serializing sources list"));

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
