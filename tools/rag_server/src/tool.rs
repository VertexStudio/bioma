use anyhow::Result;
use bioma_tool::client::ServerConfig;
use bioma_tool::ModelContextProtocolClient;
use ollama_rs::generation::tools::ToolInfo;

pub struct ToolClient {
    pub server: ServerConfig,
    pub client: ModelContextProtocolClient,
    pub tools: Vec<ToolInfo>,
}

pub struct Tools {
    pub clients: Vec<ToolClient>,
}

impl Tools {
    pub fn new() -> Self {
        Self { clients: vec![] }
    }

    pub async fn add_server(&mut self, server: ServerConfig) -> Result<()> {
        let client = ModelContextProtocolClient::new(server.clone()).await?;
        let tools = vec![];
        self.clients.push(ToolClient { server, client, tools });
        Ok(())
    }
}
