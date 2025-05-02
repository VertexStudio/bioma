use bioma_actor::{Actor, ActorId, Engine, Relay, SendOptions, SpawnOptions, SystemActorError};
use bioma_mcp::{
    schema::CallToolResult,
    server::RequestContext,
    tools::{ToolDef, ToolError},
};
use bioma_rag::prelude::{RetrieveContext, RetrieveQuery, Retriever};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::time::Duration;
use tracing::error;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct RetrieveArgs {
    #[schemars(description = "Query text to retrieve relevant context for")]
    query: String,

    #[schemars(description = "Maximum number of results to return")]
    limit: Option<usize>,

    #[schemars(description = "Filter results by source paths")]
    sources: Option<Vec<String>>,

    #[schemars(description = "Similarity threshold (0.0-1.0)")]
    threshold: Option<f32>,

    #[schemars(description = "Output format (markdown or json)")]
    format: Option<String>,
}

#[derive(Serialize)]
pub struct RetrieveTool {
    #[serde(skip_serializing)]
    id: ActorId,
    #[serde(skip_serializing)]
    engine: Engine,
}

impl RetrieveTool {
    pub async fn new(engine: &Engine) -> Result<Self, SystemActorError> {
        let id = ActorId::of::<Retriever>("/rag/retriever");

        let (mut retriever_ctx, mut retriever_actor) =
            Actor::spawn(engine.clone(), id.clone(), Retriever::default(), SpawnOptions::default()).await.map_err(
                |e| SystemActorError::LiveStream(Cow::Owned(format!("Failed to spawn retriever actor: {}", e))),
            )?;

        tokio::spawn(async move {
            if let Err(e) = retriever_actor.start(&mut retriever_ctx).await {
                error!("Retriever actor error: {}", e);
            }
        });

        Ok(Self { id, engine: engine.clone() })
    }
}

impl ToolDef for RetrieveTool {
    const NAME: &'static str = "retrieve";
    const DESCRIPTION: &'static str = "Retrieves context from indexed sources based on semantic similarity";
    type Args = RetrieveArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        let relay_id = ActorId::of::<Relay>("/rag/retriever/relay");

        let (relay_ctx, _) = Actor::spawn(self.engine.clone(), relay_id, Relay, SpawnOptions::default())
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to spawn relay: {}", e)))?;

        // Convert RetrieveArgs to RetrieveContext
        let retrieve_context = RetrieveContext::builder()
            .query(RetrieveQuery::Text(args.query))
            .limit(args.limit.unwrap_or(10))
            .threshold(args.threshold.unwrap_or(0.0))
            .sources(args.sources.unwrap_or_else(|| vec!["/global".to_string()]))
            .build();

        let response = relay_ctx
            .send_and_wait_reply::<Retriever, RetrieveContext>(
                retrieve_context,
                &self.id,
                SendOptions::builder().timeout(Duration::from_secs(200)).build(),
            )
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to retrieve context: {}", e)))?;

        // Format the response according to the requested format
        let result = match args.format.as_deref() {
            Some("json") => response.to_json(),
            _ => response.to_markdown(),
        };

        let response_value = serde_json::Value::String(result);
        Ok(CallToolResult { meta: None, content: vec![response_value], is_error: None })
    }
}
