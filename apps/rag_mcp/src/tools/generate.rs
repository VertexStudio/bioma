// apps/rag_mcp/src/tools/generate.rs
// -----------------------------------------------------------------------------
// Generate Tool – "chat + retrieval" but delegates text generation to the
// existing `Sampling` tool (server‑side). This is conceptually the same flow as
// the /chat HTTP handler, trimmed down for non‑streaming, no progress‑tracking
// use inside the MCP tool ecosystem.
// -----------------------------------------------------------------------------
// Author: Sergio & ChatGPT – May 2025

use bioma_actor::{Actor, ActorId, Relay, SendOptions, SpawnOptions};
use bioma_mcp::{
    schema::CallToolResult,
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

// Bring the chat structs into scope so we can reuse ChatQuery & friends.
use crate::chat::{ChatMessage, MessageRole};
// Bring the Sampling tool definitions.
use crate::tools::sampling::{Sampling, SamplingArgs, SamplingMessage};
// Bring server‑side Context required by Sampling.
use crate::server::Context;

#[derive(utoipa::ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ChatQuery {
    /// The conversation history as a list of messages
    pub messages: Vec<ChatMessage>,

    /// List of sources to search for relevant context
    #[schema(default = default_retriever_sources)]
    #[serde(default = "default_retriever_sources")]
    pub sources: Vec<String>,

    /// Optional schema for structured output format
    pub format: Option<chat::Schema>,

    /// List of available tools for the chat
    #[serde(default)]
    pub tools: Vec<ToolInfo>,

    /// List of tool actor identifiers
    #[serde(default)]
    pub tools_actors: Vec<String>,

    /// Whether to stream the response
    #[serde(default = "default_chat_stream")]
    pub stream: bool,

    /// Generation options
    pub options: Option<ModelOptions>,
}

fn default_retriever_sources() -> Vec<String> {
    vec!["/global".to_string()]
}

fn default_chat_stream() -> bool {
    false
}

// -----------------------------------------------------------------------------
#[derive(Serialize)]
pub struct GenerateTool {
    #[serde(skip_serializing)]
    engine: bioma_actor::Engine,
    #[serde(skip_serializing)]
    context: Context,
}

impl GenerateTool {
    /// Create a new GenerateTool. The caller (typically the tools hub actor)
    /// must pass the global Engine and a fresh Context instance.
    pub fn new(engine: &bioma_actor::Engine, context: Context) -> Self {
        Self { engine: engine.clone(), context }
    }

    /// Concatenate all *user* messages (ignoring system/assistant) to build the
    /// retriever search query – mirrors logic in the /chat handler.
    fn user_query(messages: &[ChatMessage]) -> String {
        let mut buf = String::new();
        for (i, m) in messages.iter().filter(|m| m.role == MessageRole::User).enumerate() {
            if i > 0 {
                buf.push('\n');
            }
            buf.push_str(&m.content);
        }
        buf
    }

    /// Convert ChatMessage → SamplingMessage (Sampling expects JSON‑encoded
    /// content so we wrap the raw text).
    fn to_sampling(msg: ChatMessage) -> SamplingMessage {
        SamplingMessage {
            role: msg.role.to_string().to_lowercase(),
            content: serde_json::json!({ "text": msg.content }),
        }
    }
}

impl ToolDef for GenerateTool {
    const NAME: &'static str = "generate";
    const DESCRIPTION: &'static str =
        "Generates a reply using retrieval‑augmented context and delegates LLM sampling to the `sampling` tool.";
    type Args = ChatQuery;

    async fn call(&self, args: Self::Args, _rcx: RequestContext) -> Result<CallToolResult, ToolError> {
        // ------------------------------------------------------------------
        // 1. Retrieve relevant context from RAG index.
        // ------------------------------------------------------------------
        let query_text = Self::user_query(&args.messages);
        let retrieve_ctx = RetrieveContext {
            query: RetrieveQuery::Text(query_text.clone()),
            limit: args.sources.is_empty().then_some(8).unwrap_or(args.sources.len()),
            threshold: 0.0,
            sources: args.sources.clone(),
        };

        // Spawn a relay so we can await the retriever reply without blocking.
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

        // Reverse so newer / more relevant chunks appear near the bottom (same
        // trick the chat handler uses).
        let mut chunks = retrieval.context;
        chunks.reverse();
        let context_md = retrieval.to_markdown();

        // ------------------------------------------------------------------
        // 2. Stitch together the conversation with context‑embedded system msg.
        // ------------------------------------------------------------------
        let (mut system_prompt_opt, mut convo): (Option<ChatMessage>, Vec<ChatMessage>) = (None, Vec::new());
        for m in &args.messages {
            if m.role == MessageRole::System {
                system_prompt_opt = Some(m.clone());
            } else {
                convo.push(m.clone());
            }
        }

        // Our synthesized system message comes first.
        let sys_text = match system_prompt_opt {
            Some(sys) => format!("{}\n\n{}", context_md, sys.content),
            None => format!("Use the following context to answer the user's query:\n{}", context_md),
        };
        let system_msg = ChatMessage::system(sys_text);

        // Final conversation vector in the order expected by the LLM.
        let mut full_convo = Vec::with_capacity(convo.len() + 1);
        full_convo.push(system_msg);
        full_convo.extend(convo);

        // ------------------------------------------------------------------
        // 3. Prepare SamplingArgs and invoke the Sampling tool *directly*.
        // ------------------------------------------------------------------
        let sampling_args = SamplingArgs {
            messages: full_convo.into_iter().map(Self::to_sampling).collect(),
            models_suggestions: None,
            context: None,
            max_tokens: args.options.as_ref().and_then(|o| o.max_tokens).unwrap_or(1024),
        };

        let sampling_tool = Sampling::new(self.context.clone());
        let sampling_res = sampling_tool
            .call(sampling_args, _rcx.clone())
            .await
            .map_err(|e| ToolError::Execution(format!("Sampling failed: {e}")))?;

        Ok(sampling_res)
    }
}

// -----------------------------------------------------------------------------
// Tests – quick smoke to ensure we go through the motions without panics.
// -----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{ChatMessage, MessageRole};
    use bioma_actor::Engine;

    #[tokio::test]
    async fn generate_smoke() {
        let engine = Engine::test().await.unwrap();
        let ctx = Context::test();
        let tool = GenerateTool::new(&engine, ctx);

        let query = ChatQuery {
            messages: vec![ChatMessage::user("What is Rust?".to_owned())],
            sources: vec![],
            format: None,
            tools: vec![],
            tools_actors: vec![],
            stream: false,
            options: None,
        };

        // We only assert that the call returns Ok – we can't check LLM output
        // in a unit test without hitting an external model.
        let _ = tool.call(query, RequestContext::default()).await.unwrap();
    }
}
