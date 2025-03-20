use crate::schema::{CallToolResult, TextContent};
use crate::tools::{ToolDef, ToolError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct EchoArgs {
    #[schemars(description = "The message to echo", required = true)]
    message: String,
}

#[derive(Clone, PartialEq, Debug, Serialize)]
pub struct Echo;

impl ToolDef for Echo {
    const NAME: &'static str = "echo";
    const DESCRIPTION: &'static str = "Echoes back the input message";
    type Args = EchoArgs;

    async fn call(
        &self,
        properties: Self::Args,
        client_id: String,
        client_supports_roots: bool,
    ) -> Result<CallToolResult, ToolError> {
        // TODO: We'd need:
        // 1. A way to access server's method to send requests to client.
        // 2. Who to send the request to?

        // custom logic + if client supports roots
        if smth_hpns && client_supports_roots {
            let roots = self.list_roots(client_id).await;
        }
        info!("Got roots and will do something with them: {:?}", roots);
        Ok(CallToolResult {
            content: vec![serde_json::to_value(TextContent {
                type_: "text".to_string(),
                text: properties.message,
                annotations: None,
            })
            .map_err(ToolError::ResultSerialize)?],
            is_error: Some(false),
            meta: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolDef;

    #[tokio::test]
    async fn test_echo_tool() {
        let tool = Echo;
        let props = EchoArgs { message: "hello".to_string() };

        let result = ToolDef::call(&tool, props).await.unwrap();
        assert_eq!(result.content[0]["text"].as_str().unwrap(), "hello");
        assert_eq!(result.is_error, Some(false));
    }
}
