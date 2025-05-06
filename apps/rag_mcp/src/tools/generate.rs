//! Retrieval-augmented message generation.

use std::time::Duration;

use anyhow::anyhow;
use bioma_actor::{Actor, ActorContext, ActorId, Engine, Relay, SendOptions, SpawnExistsOptions, SpawnOptions};
use bioma_llm::prelude::{ChatMessage, MessageRole};
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

type Result<T> = anyhow::Result<T>;

#[derive(Debug, Clone, Copy)]
enum ConversationalRole {
    User,
    Assistant,
}

impl TryFrom<MessageRole> for ConversationalRole {
    type Error = anyhow::Error;

    fn try_from(role: MessageRole) -> Result<Self> {
        match role {
            MessageRole::User => Ok(ConversationalRole::User),
            MessageRole::Assistant => Ok(ConversationalRole::Assistant),
            other => Err(anyhow!("Unsupported role: {:?}", other)),
        }
    }
}

impl From<ConversationalRole> for Role {
    fn from(r: ConversationalRole) -> Self {
        match r {
            ConversationalRole::User => Role::User,
            ConversationalRole::Assistant => Role::Assistant,
        }
    }
}

#[derive(Serialize)]
pub struct GenerateTool {
    #[serde(skip_serializing)]
    ctx: Context,

    #[serde(skip_serializing)]
    relay_ctx: ActorContext<Relay>,

    #[serde(skip_serializing)]
    retriever_id: ActorId,
}

impl GenerateTool {
    pub async fn new(engine: &Engine, ctx: Context) -> Result<Self> {
        const RELAY_PATH: &str = "/rag/generate/relay";
        const RETRIEVER_PATH: &str = "/rag/generate/retriever";

        let (relay_ctx, _) = Actor::spawn(
            engine.clone(),
            ActorId::of::<Relay>(RELAY_PATH),
            Relay,
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;

        let (mut rec_ctx, mut rec_actor) = Actor::spawn(
            engine.clone(),
            ActorId::of::<Retriever>(RETRIEVER_PATH),
            Retriever::default(),
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;

        tokio::spawn(async move {
            if let Err(e) = rec_actor.start(&mut rec_ctx).await {
                tracing::error!(error = %e, "Retriever actor terminated");
            }
        });

        Ok(Self { ctx, relay_ctx, retriever_id: ActorId::of::<Retriever>(RETRIEVER_PATH) })
    }

    fn user_query(messages: &[ChatMessage]) -> String {
        messages
            .iter()
            .filter(|m| m.role == MessageRole::User)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn to_sampling(msg: &ChatMessage) -> Result<SamplingMessage> {
        let conv_role: ConversationalRole = msg.role.clone().try_into()?;
        Ok(SamplingMessage { role: Role::from(conv_role).into(), content: serde_json::json!(msg.content) })
    }

    fn mk_result(text: impl Into<String>, is_error: bool) -> CallToolResult {
        let content = TextContent { type_: "text".into(), text: text.into(), annotations: None };
        CallToolResult {
            content: vec![serde_json::to_value(content).unwrap_or_default()],
            is_error: Some(is_error),
            meta: None,
        }
    }
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug)]
pub struct GenerateArgs {
    pub messages: Vec<ChatMessage>,

    #[serde(default = "default_sources")]
    pub sources: Vec<String>,
}

fn default_sources() -> Vec<String> {
    vec!["/global".into()]
}

impl ToolDef for GenerateTool {
    const NAME: &'static str = "generate";
    const DESCRIPTION: &'static str = "Generate a reply using retrieval-augmented context via direct LLM sampling.";
    type Args = GenerateArgs;

    async fn call(&self, args: Self::Args, _rc: RequestContext) -> Result<CallToolResult> {
        let query = Self::user_query(&args.messages);
        let limit = if args.sources.is_empty() { 8 } else { args.sources.len() };

        let retrieval = self
            .relay_ctx
            .send_and_wait_reply::<Retriever, _>(
                RetrieveContext {
                    query: RetrieveQuery::Text(query.clone()),
                    limit,
                    threshold: 0.0,
                    sources: args.sources.clone(),
                },
                &self.retriever_id,
                SendOptions::builder().timeout(Duration::from_secs(200)).build(),
            )
            .await?;

        let last_user_idx = args
            .messages
            .iter()
            .rposition(|m| m.role == MessageRole::User)
            .ok_or_else(|| anyhow!("No user message provided"))?;

        let context_md = retrieval.to_markdown();
        let augmented = args
            .messages
            .into_iter()
            .enumerate()
            .map(|(i, msg)| {
                if i == last_user_idx {
                    ChatMessage::user(format!(
                        "Use the following context when answering:\n{context_md}\n\n{}",
                        msg.content
                    ))
                } else {
                    msg
                }
            })
            .collect::<Vec<_>>();

        let sampling_msgs = augmented.iter().map(|m| Self::to_sampling(m)).collect::<Result<Vec<_>>>()?;

        let op = self
            .ctx
            .create_message(CreateMessageRequestParams { messages: sampling_msgs, ..Default::default() }, false)
            .await?;

        match op.await {
            Ok(msg) => Ok(Self::mk_result(format!("{:#?}", msg), false)),
            Err(e) => Ok(Self::mk_result(format!("LLM generation failed: {e}"), true)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn user_query_concat() {
        let msgs =
            vec![ChatMessage::system("sys".into()), ChatMessage::user("u1".into()), ChatMessage::user("u2".into())];
        assert_eq!(GenerateTool::user_query(&msgs), "u1\nu2");
    }

    #[test]
    fn chat_to_sampling_ok() {
        let c = ChatMessage::assistant("hi".into());
        let sm = GenerateTool::to_sampling(&c).unwrap();
        assert_eq!(sm.role, Role::Assistant.into());
        assert_eq!(sm.content, json!({ "text": "hi" }));
    }

    #[test]
    fn to_sampling_rejects_system() {
        let c = ChatMessage::system("ignored".into());
        assert!(GenerateTool::to_sampling(&c).is_err());
    }

    #[test]
    fn mk_result_variants() {
        let ok = GenerateTool::mk_result("hello", false);
        assert_eq!(ok.is_error, Some(false));

        let err = GenerateTool::mk_result("oops", true);
        assert_eq!(err.is_error, Some(true));
    }
}
