use bioma_mcp::{
    schema::{CallToolResult, TextContent},
    server::{Context, RequestContext},
    tools::{ToolDef, ToolError},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TextToRank {
    #[schemars(description = "Text to be ranked")]
    text: String,

    #[schemars(description = "Optional ID to identify this text")]
    id: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct RerankArgs {
    #[schemars(description = "Query text to compare against")]
    query: String,

    #[schemars(description = "Array of texts to rank by relevance to the query")]
    texts: Vec<TextToRank>,

    #[schemars(description = "Model to use for reranking")]
    model: Option<String>,

    #[schemars(description = "Return only top N results")]
    top_n: Option<usize>,
}

#[derive(Clone)]
pub struct Rerank {
    context: Context,
}

impl Rerank {
    pub fn new(context: Context) -> Self {
        Self { context }
    }
}

impl ToolDef for Rerank {
    const NAME: &'static str = "rerank";
    const DESCRIPTION: &'static str = "Rerank a list of texts by relevance to a query";
    type Args = RerankArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        // In a real implementation, this would connect to the reranking actor
        // For now, we'll just return mock reranked results

        let model = args.model.unwrap_or_else(|| "default-reranking-model".to_string());
        let top_n = args.top_n.unwrap_or(args.texts.len());

        // Create mock ranked results with random scores
        let mut ranked_results = args
            .texts
            .iter()
            .enumerate()
            .map(|(i, text)| {
                let id = text.id.clone().unwrap_or_else(|| format!("text-{}", i));
                let score = 1.0 - (i as f32 * 0.1); // Make the first items have higher scores

                serde_json::json!({
                    "id": id,
                    "text": text.text,
                    "score": score,
                    "rank": i + 1,
                })
            })
            .collect::<Vec<_>>();

        // Take only top_n
        ranked_results.truncate(top_n);

        let result_json = serde_json::json!({
            "query": args.query,
            "model": model,
            "total_candidates": args.texts.len(),
            "returned_candidates": ranked_results.len(),
            "results": ranked_results,
        });

        let response = serde_json::to_string_pretty(&result_json)
            .unwrap_or_else(|_| format!("Error serializing reranked results"));

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
