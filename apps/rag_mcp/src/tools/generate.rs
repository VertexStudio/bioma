// apps/rag_mcp/src/tools/generate.rs
// -----------------------------------------------------------------------------
// Generate Tool – combines retrieval with direct LLM sampling via
// `Context::create_message`. Mirrors the /chat handler but wrapped as an MCP
// tool. No custom progress tracking; we rely on the LLM operation’s own stream.
// -----------------------------------------------------------------------------
// Author: Sergio & ChatGPT – May 2025

use bioma_actor::{Actor, ActorId, Relay, SendOptions, SpawnOptions};
use bioma_mcp::{
    schema::{CallToolResult, TextContent},
    server::RequestContext,
    tools::{ToolDef, ToolError},
};
use bioma_rag::{
    prelude::{RetrieveContext, RetrieveQuery},
    retriever::Retriever,
};
use serde::Serialize;
use std::time::Duration;
use tracing::error;

// Re‑use chat types so the tool takes exactly the same payload as /chat
use crate::chat::{ChatMessage, ChatQuery, MessageRole};
// Bring Context + params for direct LLM invocation
use crate::server::{Context, CreateMessageRequestParams};
// SamplingMessage schema is used when building the LLM request
use crate::tools::sampling::SamplingMessage;

// -----------------------------------------------------------------------------
#[derive(Serialize)]
pub struct GenerateTool {
    #[serde(skip_serializing)]
    engine: bioma_actor::Engine,
    #[serde(skip_serializing)]
    context: Context,
}

impl GenerateTool {
    pub fn new(engine: &bioma_actor::Engine, context: Context) -> Self {
        Self { engine: engine.clone(), context }
    }

    fn user_query(messages: &[ChatMessage]) -> String {
        messages
            .iter()
            .filter(|m| m.role == MessageRole::User)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn to_sampling(msg: ChatMessage) -> SamplingMessage {
        SamplingMessage {
            role: msg.role.to_string().to_lowercase(),
            content: serde_json::json!({ "text": msg.content }),
        }
    }

    fn success(message: impl Into<String>) -> CallToolResult {
        CallToolResult {
            content: vec![
                serde_json::to_value(TextContent { type_: "text".into(), text: message.into(), annotations: None })
                    .unwrap_or_default(),
            ],
            is_error: Some(false),
            meta: None,
        }
    }

    fn error(msg: impl Into<String>) -> CallToolResult {
        CallToolResult {
            content: vec![
                serde_json::to_value(TextContent { type_: "text".into(), text: msg.into(), annotations: None })
                    .unwrap_or_default(),
            ],
            is_error: Some(true),
            meta: None,
        }
    }
}

impl ToolDef for GenerateTool {
    const NAME: &'static str = "generate";
    const DESCRIPTION: &'static str = "Generates a reply using retrieval‑augmented context via direct LLM sampling.";
    type Args = ChatQuery;

    async fn call(&self, args: Self::Args, request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        // 1. Retrieve context -------------------------------------------------
        let query_text = Self::user_query(&args.messages);
        let retrieve_ctx = RetrieveContext {
            query: RetrieveQuery::Text(query_text.clone()),
            limit: args.sources.is_empty().then_some(8).unwrap_or(args.sources.len()),
            threshold: 0.0,
            sources: args.sources.clone(),
        };

        let relay_id = ActorId::of::<Relay>("/rag/generate/relay");
        let (relay_ctx, _) = Actor::spawn(self.engine.clone(), relay_id, Relay, SpawnOptions::default())
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to spawn relay: {e}")))?;

        let retriever_id = ActorId::of::<Retriever>("/rag/retriever");
        let retrieval = relay_ctx
            .send_and_wait_reply::<Retriever, RetrieveContext>(
                retrieve_ctx,
                &retriever_id,
                SendOptions::builder().timeout(Duration::from_secs(200)).build(),
            )
            .await
            .map_err(|e| ToolError::Execution(format!("Retrieval failed: {e}")))?;

        let mut chunks = retrieval.context;
        chunks.reverse();
        let context_md = retrieval.to_markdown();

        // 2. Construct conversation with system prompt ----------------------
        let (mut sys_opt, mut convo): (Option<ChatMessage>, Vec<ChatMessage>) = (None, Vec::new());
        for m in &args.messages {
            if m.role == MessageRole::System {
                sys_opt = Some(m.clone());
            } else {
                convo.push(m.clone());
            }
        }

        let sys_text = match sys_opt {
            Some(sys) => format!("{}\n\n{}", context_md, sys.content),
            None => format!("Use the following context to answer the user's query:\n{}", context_md),
        };
        let system_msg = ChatMessage::system(sys_text);

        let mut full_convo = Vec::with_capacity(convo.len() + 1);
        full_convo.push(system_msg);
        full_convo.extend(convo);

        // 3. Send to LLM via Context::create_message -------------------------
        let sampling_messages: Vec<SamplingMessage> = full_convo.into_iter().map(Self::to_sampling).collect();

        let params = CreateMessageRequestParams {
            include_context: None,
            max_tokens: args.options.as_ref().and_then(|o| o.max_tokens).unwrap_or(1024),
            messages: sampling_messages,
            ..CreateMessageRequestParams::default()
        };

        let mut operation = self
            .context
            .create_message(params, true) // <- stream enabled per Sampling example
            .await
            .map_err(|e| ToolError::Execution(format!("LLM request failed: {e}")))?;

        match operation.await {
            Ok(msg) => Ok(Self::success(format!("{:#?}", msg))),
            Err(e) => Ok(Self::error(format!("LLM generation failed: {e}"))),
        }
    }
}

// -----------------------------------------------------------------------------
// Tests ------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use bioma_actor::Engine;

    #[tokio::test]
    async fn generate_smoke() {
        let engine = Engine::test().await.unwrap();
        let ctx = Context::test();
        let tool = GenerateTool::new(&engine, ctx);

        let query = ChatQuery {
            messages: vec![ChatMessage::user("Explain Rust ownership".into())],
            sources: vec![],
            format: None,
            tools: vec![],
            tools_actors: vec![],
            stream: false,
            options: None,
        };

        let _ = tool.call(query, RequestContext::default()).await;
    }
}
