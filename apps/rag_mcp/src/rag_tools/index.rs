use bioma_actor::{ActorId, Engine, Relay, SystemActorError};
use bioma_mcp::{
    schema::{CallToolResult, TextContent},
    server::RequestContext,
    tools::{ToolDef, ToolError},
};
use bioma_rag::prelude::Index as IndexArgs;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize)]
pub struct Index {
    handle: JoinHandle<()>,
    id: ActorId,
    relay: Relay,
}

impl Index {
    pub fn new(engine: &Engine) -> Result<Self, SystemActorError> {
        todo!()
    }
}

impl ToolDef for Index {
    const NAME: &'static str = "index";
    const DESCRIPTION: &'static str = "Indexes content from text or URLs for future retrieval and RAG";
    type Args = IndexArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        todo!()
    }
}
