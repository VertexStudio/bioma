use bioma_actor::{Actor, ActorId, Engine, Relay, SendOptions, SpawnOptions};
use bioma_mcp::{
    schema::CallToolResult,
    server::RequestContext,
    tools::{ToolDef, ToolError},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(JsonSchema, Debug, Serialize, Deserialize)]
pub struct UploadArgs {}

#[derive(Serialize)]
pub struct UploadTool {
    #[serde(skip_serializing)]
    engine: Engine,
}

#[derive(Serialize)]
struct Uploaded {
    message: String,
    paths: Vec<PathBuf>,
    size: usize,
}

impl UploadTool {
    pub fn new(engine: Engine) -> Self {
        Self { engine }
    }
}

impl ToolDef for UploadTool {
    const NAME: &'static str = "upload";
    const DESCRIPTION: &'static str = "Upload files for indexing and retrieval";
    type Args = UploadArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, ToolError> {
        todo!()
    }
}
