use anyhow::Error;
use bioma_actor::{Actor, ActorId, Relay, SendOptions, SpawnExistsOptions, SpawnOptions};
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
use std::time::Duration;

#[derive(Serialize)]
pub struct GenerateTool {
    #[serde(skip_serializing)]
    context: Context,
    #[serde(skip_serializing)]
    relay_ctx: bioma_actor::ActorContext<Relay>,
    #[serde(skip_serializing)]
    retriever_id: ActorId,
}

impl GenerateTool {
    pub async fn new(engine: &bioma_actor::Engine, context: Context) -> Result<Self, Error> {
        let relay_id = ActorId::of::<Relay>("/rag/generate/relay");
        let retriever_id = ActorId::of::<Retriever>("/rag/generate/retriever");

        let (relay_ctx, _) = Actor::spawn(
            engine.clone(),
            relay_id.clone(),
            Relay,
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;

        let (mut retriever_ctx, mut retriever_actor) = Actor::spawn(
            engine.clone(),
            retriever_id.clone(),
            Retriever::default(),
            SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
        )
        .await?;

        tokio::spawn(async move {
            if let Err(e) = retriever_actor.start(&mut retriever_ctx).await {
                tracing::error!("Retriever actor error: {}", e);
            }
        });

        Ok(Self { context, relay_ctx, retriever_id })
    }

    fn user_query(messages: &[ChatMessage]) -> String {
        messages
            .iter()
            .filter(|m| m.role == MessageRole::User)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn to_sampling(msg: ChatMessage) -> Result<SamplingMessage, anyhow::Error> {
        let role = match msg.role {
            MessageRole::User => Role::User,
            MessageRole::Assistant => Role::Assistant,
            _ => return Err(anyhow::anyhow!("Unsupported message role: {:?}", msg.role)),
        };
        Ok(SamplingMessage { role: role.into(), content: serde_json::json!({ "text": msg.content }) })
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

#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug)]
pub struct GenerateArgs {
    pub messages: Vec<ChatMessage>,

    #[serde(default = "default_retriever_sources")]
    pub sources: Vec<String>,
}

fn default_retriever_sources() -> Vec<String> {
    vec!["/global".to_string()]
}

impl ToolDef for GenerateTool {
    const NAME: &'static str = "generate";
    const DESCRIPTION: &'static str = "Generates a reply using retrieval‑augmented context via direct LLM sampling.";
    type Args = GenerateArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, Error> {
        println!("args: {:#?}", args);
        let query_text = Self::user_query(&args.messages);
        let retrieve_ctx = RetrieveContext {
            query: RetrieveQuery::Text(query_text.clone()),
            limit: args.sources.is_empty().then_some(8).unwrap_or(args.sources.len()),
            threshold: 0.0,
            sources: args.sources.clone(),
        };

        tracing::info!("retrieve_ctx: {:#?}", retrieve_ctx);

        let retrieval = self
            .relay_ctx
            .send_and_wait_reply::<Retriever, RetrieveContext>(
                retrieve_ctx,
                &self.retriever_id,
                SendOptions::builder().timeout(Duration::from_secs(200)).build(),
            )
            .await?;

        tracing::info!("retrieval: {:#?}", retrieval);

        let mut chunks = retrieval.context.clone();
        chunks.reverse();
        let context_md = retrieval.to_markdown();

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

        let sampling_messages: Vec<SamplingMessage> =
            full_convo.into_iter().map(Self::to_sampling).collect::<Result<_, _>>().unwrap();

        let params =
            CreateMessageRequestParams { messages: sampling_messages, ..CreateMessageRequestParams::default() };

        let operation = self.context.create_message(params, false).await?;

        match operation.await {
            Ok(msg) => Ok(Self::success(format!("{:#?}", msg))),
            Err(e) => Ok(Self::error(format!("LLM generation failed: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn user_query() {
        let messages = vec![
            ChatMessage::system("ignore me".into()),
            ChatMessage::user("what's up?".into()),
            ChatMessage::assistant("hello!".into()),
            ChatMessage::user("any updates?".into()),
        ];
        let q = GenerateTool::user_query(&messages);
        assert_eq!(q, "what's up?\nany updates?");
    }

    #[test]
    fn to_sampling() {
        let u = ChatMessage::user("hey".into());
        let sm_u = GenerateTool::to_sampling(u.clone()).unwrap();
        assert_eq!(sm_u.role, Role::User.into());
        assert_eq!(sm_u.content["text"], Value::String("hey".into()));

        let a = ChatMessage::assistant("hi there".into());
        let sm_a = GenerateTool::to_sampling(a.clone()).unwrap();
        assert_eq!(sm_a.role, Role::Assistant.into());
        assert_eq!(sm_a.content["text"], Value::String("hi there".into()));
    }

    #[test]
    fn rejects_roles() {
        let sys = ChatMessage::system("system prompt".into());
        assert!(GenerateTool::to_sampling(sys).is_err());
    }
}
