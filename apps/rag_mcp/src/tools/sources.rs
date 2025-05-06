use anyhow::Error;
use bioma_actor::{Actor, ActorId, Engine, Relay, SendOptions, SpawnExistsOptions, SpawnOptions, SystemActorError};
use bioma_mcp::{schema::CallToolResult, server::RequestContext, tools::ToolDef};
use bioma_rag::prelude::{ListSources, Retriever};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::time::Duration;
use tracing::error;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ListSourcesArgs {}

#[derive(Serialize)]
pub struct SourcesTool {
    #[serde(skip_serializing)]
    id: ActorId,
    #[serde(skip_serializing)]
    engine: Engine,
}

impl SourcesTool {
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

impl ToolDef for SourcesTool {
    const NAME: &'static str = "list_sources";
    const DESCRIPTION: &'static str = "List indexed sources available for retrieval";
    type Args = ListSourcesArgs;

    async fn call(&self, _args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, Error> {
        let relay_id = ActorId::of::<Relay>("/rag_mcp/retriever/relay");

        let (relay_ctx, _) = Actor::spawn(
            self.engine.clone(),
            relay_id,
            Relay,
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;

        let response = relay_ctx
            .send_and_wait_reply::<Retriever, ListSources>(
                ListSources,
                &self.id,
                SendOptions::builder().timeout(Duration::from_secs(60)).build(),
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
        indexer::{Index as IndexArgs, IndexContent, TextChunkConfig, TextsContent},
        prelude::ListedSources,
    };

    #[tokio::test]
    async fn list_sources_after_indexing() {
        let engine = Engine::test().await.unwrap();

        let source_name = "unit-test-source";
        let index_tool = IndexTool::new(&engine).await.unwrap();
        index_tool
            .call(
                IndexArgs {
                    content: IndexContent::Texts(TextsContent {
                        texts: vec!["Sergio loves type-driven dev.".to_owned()],
                        mime_type: "text/plain".into(),
                        config: TextChunkConfig::default(),
                    }),
                    source: source_name.into(),
                    summarize: false,
                },
                RequestContext::default(),
            )
            .await
            .expect("indexing must succeed");

        let list_tool = SourcesTool::new(&engine).await.unwrap();
        let raw = list_tool.call(ListSourcesArgs {}, RequestContext::default()).await.expect("listing must succeed");

        let listed: ListedSources = serde_json::from_value(raw.content[0].clone()).unwrap();

        assert!(
            listed.sources.iter().any(|s| s.source == source_name),
            "returned list should include the source we just indexed"
        );

        assert!(listed.sources.iter().all(|s| !s.uri.is_empty()));
    }
}
