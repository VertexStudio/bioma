use anyhow::Error;
use bioma_actor::{Actor, ActorId, Engine, SendOptions, SystemActorError};
use bioma_mcp::{schema::CallToolResult, server::RequestContext, tools::ToolDef};
use bioma_rag::prelude::{Index as IndexArgs, Indexer};
use serde::Serialize;
use std::borrow::Cow;
use std::time::Duration;
use tracing::error;

use crate::tools::ToolRelay;

#[derive(Serialize)]
pub struct IndexTool {
    #[serde(skip_serializing)]
    id: ActorId,
    #[serde(skip_serializing)]
    relay: ToolRelay,
}

impl IndexTool {
    pub async fn new(engine: &Engine) -> Result<Self, SystemActorError> {
        let id = ActorId::of::<Indexer>("/rag_mcp/indexer");

        let (mut indexer_ctx, mut indexer_actor) = Actor::spawn(
            engine.clone(),
            id.clone(),
            Indexer::default(),
            bioma_actor::SpawnOptions::builder().exists(bioma_actor::SpawnExistsOptions::Reset).build(),
        )
        .await
        .map_err(|e| SystemActorError::LiveStream(Cow::Owned(format!("Failed to spawn indexer: {}", e))))?;

        tokio::spawn(async move {
            if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
                error!("Indexer actor error: {}", e);
            }
        });

        let relay = ToolRelay::new(engine, "/tool_relay/rag_mcp/indexer").await?;

        Ok(Self { id, relay })
    }
}

impl ToolDef for IndexTool {
    const NAME: &'static str = "index";
    const DESCRIPTION: &'static str = "Indexes content from text or URLs for future retrieval and RAG";
    type Args = IndexArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, Error> {
        let response = self
            .relay
            .ctx
            .send_and_wait_reply::<Indexer, IndexArgs>(
                args,
                &self.id,
                SendOptions::builder().timeout(Duration::from_secs(600)).build(),
            )
            .await?;

        let response_value = serde_json::to_value(response)?;

        Ok(CallToolResult { meta: None, content: vec![response_value], is_error: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ingest::{self, IngestTool};
    use bioma_rag::{
        indexer::{IndexStatus, Indexed, TextsContent},
        prelude::{GlobsContent, Index as IndexArgs, IndexContent, TextChunkConfig},
    };
    use std::sync::Arc;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    #[tokio::test]
    async fn index_plain_text() {
        let engine = Engine::test().await.unwrap();
        let index_tool = IndexTool::new(&engine).await.unwrap();

        let args = IndexArgs {
            content: IndexContent::Texts(TextsContent {
                texts: vec!["Hello world!".to_owned()],
                mime_type: "text/plain".into(),
                config: TextChunkConfig::default(),
            }),
            source: "unit-test".into(),
            summarize: false,
        };
        let res = index_tool.call(args, RequestContext::default()).await.expect("indexing succeeds");

        let indexed: Indexed = serde_json::from_value(res.content[0].clone()).expect("valid Indexed result");
        assert!(indexed.indexed >= 1, "Expected at least one indexed item");
        assert!(!indexed.sources.is_empty(), "Expected sources to be non-empty");
    }

    #[tokio::test]
    async fn upload_then_index_file() {
        let server = MockServer::start().await;
        let file_body = b"File to be indexed";
        Mock::given(method("GET"))
            .and(path("/docs/file.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(file_body))
            .mount(&server)
            .await;

        let engine = Engine::test().await.unwrap();

        let ingest_tool = IngestTool::new(Arc::new(engine.clone()));
        let ingest_args =
            ingest::IngestArgs { url: format!("{}/docs/file.txt", server.uri()), path: "docs/file.txt".into() };
        ingest_tool.call(ingest_args, RequestContext::default()).await.expect("upload works");

        let index_tool = IndexTool::new(&engine).await.unwrap();
        let idx_args = IndexArgs {
            content: IndexContent::Globs(GlobsContent {
                globs: vec!["docs/*.txt".into()],
                config: TextChunkConfig::default(),
            }),
            source: "unit-test-glob".into(),
            summarize: false,
        };
        let res = index_tool.call(idx_args, RequestContext::default()).await.expect("indexing works");

        let indexed: Indexed = serde_json::from_value(res.content[0].clone()).expect("valid Indexed result");
        assert_eq!(indexed.indexed, 1, "Expected one indexed file");
        assert_eq!(indexed.sources.len(), 1, "Expected one source");
        assert!(matches!(indexed.sources[0].status, IndexStatus::Indexed), "Expected status to be Indexed");
    }
}
