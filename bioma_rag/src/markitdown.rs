use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{error, info};
use url::Url;

#[derive(Deserialize)]
struct MarkitDownServerResponse {
    text_content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzeMCFile {
    pub file_path: PathBuf,
}

#[derive(thiserror::Error, Debug)]
pub enum MarkitDownError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Error get file")]
    ErrorFile(#[from] std::io::Error),
    #[error("Error post file to Markitdown")]
    ErrorPostFile(#[from] reqwest::Error),
    #[error("Error serde data")]
    SerdeJson(#[from] serde_json::Error),
}

impl ActorError for MarkitDownError {}

#[derive(bon::Builder, Debug, Serialize, Deserialize, Clone)]
pub struct MarkitDown {
    #[builder(default = Url::parse("http://localhost:5001").unwrap())]
    pub markitdown_url: Url,
}

impl Default for MarkitDown {
    fn default() -> Self {
        Self::builder().build()
    }
}

impl MarkitDown {
    async fn post_markitdown(&self, req: &AnalyzeMCFile) -> Result<String, MarkitDownError> {
        let form_result = reqwest::multipart::Form::new().file("file", &req.file_path).await;

        match form_result {
            Ok(form) => {
                let markitdown_url =
                    self.markitdown_url.clone().join("/convert").expect("Failed to join markitdown URL");
                let response = reqwest::Client::new().post(markitdown_url).multipart(form).send().await;

                match response {
                    Ok(resp) => {
                        let json_response = serde_json::from_str::<MarkitDownServerResponse>(&resp.text().await?)?;

                        Ok(json_response.text_content)
                    }
                    Err(error) => Err(MarkitDownError::ErrorPostFile(error)),
                }
            }
            Err(error) => Err(MarkitDownError::ErrorFile(error)),
        }
    }
}

impl Message<AnalyzeMCFile> for MarkitDown {
    type Response = String;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, msg: &AnalyzeMCFile) -> Result<(), MarkitDownError> {
        info!("path {:?}", msg.file_path);
        let markdown = self.post_markitdown(msg).await?;
        ctx.reply(markdown).await?;
        Ok(())
    }
}

impl Actor for MarkitDown {
    type Error = MarkitDownError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("{} Started", ctx.id());

        let mut stream = ctx.recv().await?;

        while let Some(Ok(frame)) = stream.next().await {
            if let Some(input) = frame.is::<AnalyzeMCFile>() {
                let response = self.reply(ctx, &input, &frame).await;
                if let Err(err) = response {
                    error!("{} {:?}", ctx.id(), err);
                }
            }
        }

        info!("{} Finished", ctx.id());
        Ok(())
    }
}
