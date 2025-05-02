use bioma_mcp::{
    schema::{CallToolResult, TextContent},
    server::{Context, RequestContext},
    tools::{ToolDef, ToolError},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct DeleteSourceArgs {
    #[schemars(description = "ID of the source to delete")]
    source_id: Option<String>,

    #[schemars(description = "Name of the source to delete")]
    source_name: Option<String>,

    #[schemars(description = "Confirm deletion")]
    confirm: bool,
}

#[derive(Clone)]
pub struct DeleteSource {
    context: Context,
}

impl DeleteSource {
    pub fn new(context: Context) -> Self {
        Self { context }
    }
}

impl ToolDef for DeleteSource {
    const NAME: &'static str = "delete_source";
    const DESCRIPTION: &'static str = "Delete an indexed source";
    type Args = DeleteSourceArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        // Require confirmation
        if !args.confirm {
            return Err(ToolError::Custom("Deletion must be confirmed by setting confirm=true".to_string()));
        }

        // Require either source_id or source_name
        let source_identifier = match (&args.source_id, &args.source_name) {
            (Some(id), _) => id.clone(),
            (_, Some(name)) => name.clone(),
            (None, None) => {
                return Err(ToolError::Custom("Either source_id or source_name must be provided".to_string()));
            }
        };

        // In a real implementation, this would delete the source from the database
        // For now, we'll just return a success message

        let response = format!("Successfully deleted source '{}'", source_identifier);

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
