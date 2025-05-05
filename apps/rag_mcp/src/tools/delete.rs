use anyhow::Error;
use bioma_actor::{Actor, ActorId, Engine, Relay, SendOptions, SpawnOptions, SystemActorError};
use bioma_mcp::{schema::CallToolResult, server::RequestContext, tools::ToolDef};
use bioma_rag::prelude::{DeleteSource as DeleteSourceArgs, Indexer};
use serde::Serialize;
use std::borrow::Cow;
use std::time::Duration;
use tracing::error;

#[derive(Serialize)]
pub struct DeleteTool {
    #[serde(skip_serializing)]
    id: ActorId,
    #[serde(skip_serializing)]
    engine: Engine,
}

impl DeleteTool {
    pub async fn new(engine: &Engine) -> Result<Self, SystemActorError> {
        let id = ActorId::of::<Indexer>("/rag/delete");

        let (mut indexer_ctx, mut indexer_actor) =
            Actor::spawn(engine.clone(), id.clone(), Indexer::default(), SpawnOptions::default()).await.map_err(
                |e| SystemActorError::LiveStream(Cow::Owned(format!("Failed to spawn indexer actor: {}", e))),
            )?;

        tokio::spawn(async move {
            if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
                error!("Indexer actor error: {}", e);
            }
        });

        Ok(Self { id, engine: engine.clone() })
    }
}

impl ToolDef for DeleteTool {
    const NAME: &'static str = "delete_source";
    const DESCRIPTION: &'static str = "Delete indexed sources and their associated embeddings";
    type Args = DeleteSourceArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, Error> {
        let relay_id = ActorId::of::<Relay>("/rag/delete/relay");

        let (relay_ctx, _) = Actor::spawn(self.engine.clone(), relay_id, Relay, SpawnOptions::default()).await?;

        let delete_source = DeleteSourceArgs { sources: args.sources, delete_from_disk: args.delete_from_disk };

        let response = relay_ctx
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
