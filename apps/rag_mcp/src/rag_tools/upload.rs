use bioma_mcp::{
    schema::{CallToolResult, TextContent},
    server::{Context, RequestContext},
    tools::{ToolDef, ToolError},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct UploadArgs {
    #[schemars(description = "Base64 encoded file content")]
    file_content: String,

    #[schemars(description = "Filename with extension")]
    filename: String,

    #[schemars(description = "Auto-index the file after upload")]
    auto_index: Option<bool>,

    #[schemars(description = "Source name to use if auto-indexing")]
    source_name: Option<String>,
}

#[derive(Clone)]
pub struct Upload {
    context: Context,
    base_dir: PathBuf,
}

impl Upload {
    pub fn new(context: Context, base_dir: PathBuf) -> Self {
        Self { context, base_dir }
    }
}

impl ToolDef for Upload {
    const NAME: &'static str = "upload";
    const DESCRIPTION: &'static str = "Upload files for indexing and retrieval";
    type Args = UploadArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        // In a real implementation, this would:
        // 1. Decode the base64 content
        // 2. Save the file to the uploads directory
        // 3. Optionally trigger indexing

        let auto_index = args.auto_index.unwrap_or(false);
        let file_size = args.file_content.len();

        // Mock upload result
        let upload_dir = self.base_dir.join("uploads");
        let file_path = upload_dir.join(&args.filename);

        let result = serde_json::json!({
            "message": format!("Successfully uploaded {} (size: {} bytes)", args.filename, file_size),
            "path": file_path.to_string_lossy(),
            "size": file_size,
            "auto_indexed": auto_index,
        });

        let response =
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| format!("Error serializing upload result"));

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
