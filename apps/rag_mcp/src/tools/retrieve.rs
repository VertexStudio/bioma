use anyhow::Error;
use bioma_actor::{Actor, ActorId, Engine, Relay, SendOptions, SpawnExistsOptions, SpawnOptions, SystemActorError};
use bioma_mcp::{schema::CallToolResult, server::RequestContext, tools::ToolDef};
use bioma_rag::prelude::{RetrieveContext as RetrieveContextArgs, Retriever};
use serde::Serialize;
use std::borrow::Cow;
use std::time::Duration;
use tracing::error;

#[derive(Serialize)]
pub struct RetrieveTool {
    #[serde(skip_serializing)]
    id: ActorId,
    #[serde(skip_serializing)]
    engine: Engine,
}

impl RetrieveTool {
    pub async fn new(engine: &Engine) -> Result<Self, SystemActorError> {
        let id = ActorId::of::<Retriever>("/rag_mcp/retriever");

        let (mut retriever_ctx, mut retriever_actor) = Actor::spawn(
            engine.clone(),
            id.clone(),
            Retriever::default(),
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await
        .map_err(|e| SystemActorError::LiveStream(Cow::Owned(format!("Failed to spawn retriever actor: {}", e))))?;

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
    type Args = RetrieveContextArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, Error> {
        let relay_id = ActorId::of::<Relay>("/rag_mcp/retriever/relay");

        let (relay_ctx, _relay_actor) = Actor::spawn(
            self.engine.clone(),
            relay_id,
            Relay,
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;

        let response = relay_ctx
            .send_and_wait_reply::<Retriever, RetrieveContextArgs>(
                args,
                &self.id,
                SendOptions::builder().timeout(Duration::from_secs(200)).build(),
            )
            .await?;

        let response_value = serde_json::to_value(response)?;

        Ok(CallToolResult { meta: None, content: vec![response_value], is_error: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::index::IndexTool;
    use bioma_mcp::server::RequestContext;
    use bioma_rag::{
        indexer::TextsContent,
        prelude::{
            Index as IndexArgs, IndexContent, RetrieveContext as RetrieveContextArgs, RetrieveQuery, TextChunkConfig,
        },
        retriever::RetrievedContext,
    };

    #[tokio::test]
    async fn index_then_retrieve() {
        let engine = Engine::test().await.unwrap();

        let idx_tool = IndexTool::new(&engine).await.unwrap();
        idx_tool
            .call(
                IndexArgs {
                    content: IndexContent::Texts(TextsContent {
                        texts: vec!["Rust guarantees memory-safety.".to_owned()],
                        mime_type: "text/plain".into(),
                        config: TextChunkConfig::default(),
                    }),
                    source: "/test-source".into(),
                    summarize: false,
                },
                RequestContext::default(),
            )
            .await
            .expect("indexing should succeed");

        let ret_tool = RetrieveTool::new(&engine).await.unwrap();

        let raw = ret_tool
            .call(
                RetrieveContextArgs {
                    query: RetrieveQuery::Text("memory safety".into()),
                    limit: 5,
                    threshold: 0.0,
                    sources: vec!["/test-source".into()],
                },
                RequestContext::default(),
            )
            .await
            .expect("retrieval should succeed");

        let retrieved: RetrievedContext = serde_json::from_value(raw.content[0].clone()).unwrap();

        assert!(!retrieved.context.is_empty(), "at least one context must be returned");

        let ctxt = &retrieved.context[0];
        let text = ctxt.text.as_deref().unwrap_or_default();
        assert!(
            text.contains("memory") || text.contains("Rust"),
            "retrieved text should relate to the indexed sentence"
        );
    }
}
