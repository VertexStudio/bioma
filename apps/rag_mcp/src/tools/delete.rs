use anyhow::Error;
use bioma_actor::{Actor, ActorId, Engine, SendOptions, SpawnExistsOptions, SpawnOptions};
use bioma_mcp::{schema::CallToolResult, server::RequestContext, tools::ToolDef};
use bioma_rag::prelude::{DeleteSource as DeleteSourceArgs, Indexer};
use serde::Serialize;
use std::time::Duration;
use tracing::error;

use crate::tools::ToolRelay;

#[derive(Serialize)]
pub struct DeleteTool {
    #[serde(skip_serializing)]
    id: ActorId,
    #[serde(skip_serializing)]
    relay: ToolRelay,
}

impl DeleteTool {
    pub async fn new(engine: &Engine) -> Result<Self, Error> {
        let id = ActorId::of::<Indexer>("/rag_mcp/delete");

        let (mut indexer_ctx, mut indexer_actor) = Actor::spawn(
            engine.clone(),
            id.clone(),
            Indexer::default(),
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;

        tokio::spawn(async move {
            if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
                error!("Indexer actor error: {}", e);
            }
        });

        let relay = ToolRelay::new(engine, "/tool_relay/rag_mcp/delete").await?;

        Ok(Self { id, relay })
    }
}

impl ToolDef for DeleteTool {
    const NAME: &'static str = "delete_source";
    const DESCRIPTION: &'static str = "Delete indexed sources and their associated embeddings";
    type Args = DeleteSourceArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, Error> {
        let delete_source = DeleteSourceArgs { sources: args.sources, delete_from_disk: args.delete_from_disk };

        let response = self
            .relay
            .ctx
            .send_and_wait_reply::<Indexer, DeleteSourceArgs>(
                delete_source,
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
        indexer::{Index as IndexArgs, IndexContent, TextChunkConfig, TextsContent},
        prelude::{DeleteSource as DeleteSourceArgs, DeletedSource},
    };

    #[tokio::test]
    async fn index_then_delete_source() {
        let engine = Engine::test().await.unwrap();

        let source_name = "/unit-test-delete";
        let index_tool = IndexTool::new(&engine).await.unwrap();
        index_tool
            .call(
                IndexArgs {
                    content: IndexContent::Texts(TextsContent {
                        texts: vec!["This text will be deleted.".to_owned()],
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

        let delete_tool = DeleteTool::new(&engine).await.unwrap();
        let raw = delete_tool
            .call(
                DeleteSourceArgs { sources: vec![source_name.into()], delete_from_disk: false },
                RequestContext::default(),
            )
            .await
            .expect("deletion must succeed");

        let deleted: DeletedSource = serde_json::from_value(raw.content[0].clone()).unwrap();

        assert!(deleted.deleted_embeddings >= 1, "at least one embedding should be removed");
        assert!(
            deleted.deleted_sources.iter().any(|cs| cs.source == source_name),
            "returned list should include the deleted source label"
        );
    }
}
