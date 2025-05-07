use anyhow::{Error, anyhow};
use bioma_actor::{Actor, ActorId, Engine, SendOptions, SpawnExistsOptions, SpawnOptions};
use bioma_mcp::{
    schema::{CallToolResult, CreateMessageRequestParams, Role, SamplingMessage, TextContent},
    server::{Context, RequestContext},
    tools::ToolDef,
};
use bioma_rag::{
    prelude::{RetrieveContext, RetrieveQuery},
    retriever::Retriever,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::time::Duration;

use crate::tools::ToolRelay;

type UserQuery = String;
type ContextMarkdown = String;

#[derive(Serialize)]
pub struct GenerateTool {
    #[serde(skip_serializing)]
    retriever_id: ActorId,

    #[serde(skip_serializing)]
    relay: ToolRelay,

    #[serde(skip_serializing)]
    ctx: Context,
}

impl GenerateTool {
    pub async fn new(engine: &Engine, ctx: Context) -> Result<Self, Error> {
        let (mut rec_ctx, mut rec_actor) = Actor::spawn(
            engine.clone(),
            ActorId::of::<Retriever>("rag/generate/retriever"),
            Retriever::default(),
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;

        tokio::spawn(async move {
            if let Err(e) = rec_actor.start(&mut rec_ctx).await {
                tracing::error!(error = %e, "Retriever actor terminated");
            }
        });

        let relay = ToolRelay::new(engine, "/tool_relay/rag/generate").await?;

        Ok(Self { ctx, relay, retriever_id: ActorId::of::<Retriever>("rag/generate/retriever") })
    }

    fn extract_text(content: &JsonValue) -> Option<&str> {
        match content {
            JsonValue::String(s) => Some(s.as_str()),
            JsonValue::Object(map) => map.get("text").and_then(|v| v.as_str()),
            _ => None,
        }
    }

    fn extract_user_query(messages: &[SamplingMessage]) -> UserQuery {
        messages
            .iter()
            .filter(|m| m.role == Role::User)
            .filter_map(|m| Self::extract_text(&m.content))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn create_result(text: impl Into<String>, is_error: bool) -> CallToolResult {
        let content = TextContent { type_: "text".into(), text: text.into(), annotations: None };

        CallToolResult {
            content: vec![serde_json::to_value(content).unwrap_or_default()],
            is_error: Some(is_error),
            meta: None,
        }
    }

    fn add_context_to_system_prompt(request: &mut CreateMessageRequestParams, context: &ContextMarkdown) {
        const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant.";

        let context_section = format!("Use the following context when answering:\n{context}\n\n");

        if let Some(current_prompt) = request.system_prompt.as_mut() {
            *current_prompt = format!("{context_section}{current_prompt}");
        } else {
            request.system_prompt = Some(format!("{context_section}{DEFAULT_SYSTEM_PROMPT}"));
        }
    }

    async fn retrieve_context(
        &self,
        query: UserQuery,
        sources: Vec<String>,
        limit: usize,
    ) -> Result<ContextMarkdown, Error> {
        let retrieval = self
            .relay
            .ctx
            .send_and_wait_reply::<Retriever, RetrieveContext>(
                RetrieveContext { query: RetrieveQuery::Text(query), limit, threshold: 0.0, sources },
                &self.retriever_id,
                SendOptions::builder().timeout(Duration::from_secs(200)).build(),
            )
            .await?;

        Ok(retrieval.to_markdown())
    }

    async fn generate_message(&self, request: CreateMessageRequestParams) -> Result<String, Error> {
        let operation = self.ctx.create_message(request, false).await?;

        match operation.await {
            Ok(msg) => Ok(format!("{:#?}", msg)),
            Err(e) => Err(anyhow!("LLM generation failed: {}", e)),
        }
    }
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug)]
pub struct GenerateArgs {
    #[serde(flatten)]
    pub create_message: CreateMessageRequestParams,

    #[serde(default = "default_sources")]
    pub sources: Vec<String>,
}

fn default_sources() -> Vec<String> {
    vec!["/global".into()]
}

impl ToolDef for GenerateTool {
    const NAME: &'static str = "generate";
    const DESCRIPTION: &'static str = "Generate a reply using retrieval‑augmented context via direct LLM sampling.";
    type Args = GenerateArgs;

    async fn call(&self, args: Self::Args, _rc: RequestContext) -> Result<CallToolResult, Error> {
        let mut req = args.create_message;
        let msgs = &req.messages;

        let query = Self::extract_user_query(msgs);
        if query.is_empty() {
            return Ok(Self::create_result("No user content found – nothing to do", true));
        }

        let context = match self.retrieve_context(query, args.sources.clone(), 10).await {
            Ok(context) => context,
            Err(e) => return Ok(Self::create_result(format!("Retrieval failed: {e}"), true)),
        };

        Self::add_context_to_system_prompt(&mut req, &context);

        match self.generate_message(req).await {
            Ok(result) => Ok(Self::create_result(result, false)),
            Err(e) => Ok(Self::create_result(e.to_string(), true)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_message(role: Role, text: &str) -> SamplingMessage {
        SamplingMessage { role, content: JsonValue::String(text.to_owned()) }
    }

    fn create_test_request(
        system_prompt: Option<String>,
        messages: Vec<SamplingMessage>,
    ) -> CreateMessageRequestParams {
        CreateMessageRequestParams { system_prompt, messages, max_tokens: 100, ..Default::default() }
    }

    #[test]
    fn test_user_query_concatenation() {
        let messages = vec![
            create_test_message(Role::Assistant, "ignore"),
            create_test_message(Role::User, "one"),
            create_test_message(Role::User, "two"),
        ];
        assert_eq!(GenerateTool::extract_user_query(&messages), "one\ntwo");
    }

    #[test]
    fn test_add_context_to_existing_system_prompt() {
        let messages = vec![create_test_message(Role::User, "test query")];
        let mut request = create_test_request(Some("Original system prompt".to_string()), messages);
        let context = "Retrieved context";

        GenerateTool::add_context_to_system_prompt(&mut request, &context.to_string());

        let expected = "Use the following context when answering:\nRetrieved context\n\nOriginal system prompt";
        assert_eq!(request.system_prompt, Some(expected.to_string()));
    }

    #[test]
    fn test_add_context_to_no_system_prompt() {
        let messages = vec![create_test_message(Role::User, "test query")];
        let mut request = create_test_request(None, messages);
        let context = "Retrieved context";

        GenerateTool::add_context_to_system_prompt(&mut request, &context.to_string());

        let expected = "Use the following context when answering:\nRetrieved context\n\nYou are a helpful assistant.";
        assert_eq!(request.system_prompt, Some(expected.to_string()));
    }
}
