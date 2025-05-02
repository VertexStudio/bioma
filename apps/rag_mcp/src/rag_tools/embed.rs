use bioma_mcp::{
    schema::{CallToolResult, TextContent},
    server::{Context, RequestContext},
    tools::{ToolDef, ToolError},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct EmbedArgs {
    #[schemars(description = "Text to embed")]
    text: String,

    #[schemars(description = "Model to use for embeddings")]
    model: Option<String>,

    #[schemars(description = "Whether to normalize the embeddings")]
    normalize: Option<bool>,
}

#[derive(Clone)]
pub struct Embed {
    context: Context,
}

impl Embed {
    pub fn new(context: Context) -> Self {
        Self { context }
    }
}

impl ToolDef for Embed {
    const NAME: &'static str = "embed";
    const DESCRIPTION: &'static str = "Generate embeddings for text";
    type Args = EmbedArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        // In a real implementation, this would connect to the embeddings actor
        // For now, we'll just return mock embeddings

        let model = args.model.unwrap_or_else(|| "default-embeddings-model".to_string());
        let normalize = args.normalize.unwrap_or(true);

        // Mock embeddings - in reality this would be much larger
        let mock_embeddings = vec![0.1, 0.2, 0.3, 0.4, 0.5];

        let embedding_json = serde_json::json!({
            "model": model,
            "text": args.text,
            "normalized": normalize,
            "dimensions": mock_embeddings.len(),
            "embedding": mock_embeddings,
        });

        let response =
            serde_json::to_string_pretty(&embedding_json).unwrap_or_else(|_| format!("Error serializing embeddings"));

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
