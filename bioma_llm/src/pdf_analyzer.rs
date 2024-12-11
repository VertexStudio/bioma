use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{error, info};
use url::Url;

#[derive(Deserialize)]
struct JsonDataFromPdf {
    text: String,
    #[serde(rename = "type")]
    item_type: String,
}

fn convert_pdf_json_to_markdown(json_data: &Vec<JsonDataFromPdf>) -> Result<String, PdfAnalyzerError> {
    let mut markdown = String::new();

    for item in json_data {
        match item.item_type.as_str() {
            "Title" => markdown.push_str(&format!("# {}", item.text)),
            "Section header" => markdown.push_str(&format!("# {}", item.text)),
            "Text" => markdown.push_str(&format!("{}", item.text)),
            "Table" => markdown.push_str(""),
            _ => markdown.push_str(&format!("{}", item.text)),
        }
        markdown.push_str("\n");
    }

    Ok(markdown)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzePdf {
    pub file_path: PathBuf,
}

#[derive(thiserror::Error, Debug)]
pub enum PdfAnalyzerError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Error get file")]
    ErrorFile(#[from] std::io::Error),
    #[error("Error post file to pdf-analyzer")]
    ErrorPostFile(#[from] reqwest::Error),
    #[error("Error serde data")]
    SerdeJson(#[from] serde_json::Error),
}

impl ActorError for PdfAnalyzerError {}

#[derive(bon::Builder, Debug, Serialize, Deserialize, Clone)]
pub struct PdfAnalyzer {
    #[builder(default = Url::parse("http://localhost:5060").unwrap())]
    pub pdf_analyzer_url: Url,
}

impl Default for PdfAnalyzer {
    fn default() -> Self {
        Self::builder().build()
    }
}

impl PdfAnalyzer {
    async fn post_pdf_analyzer(&self, file_path: &Path) -> Result<String, PdfAnalyzerError> {
        let form_result = reqwest::multipart::Form::new().file("file", file_path).await;

        match form_result {
            Ok(form) => {
                let response = reqwest::Client::new().post(self.pdf_analyzer_url.clone()).multipart(form).send().await;

                match response {
                    Ok(resp) => {
                        let json_response = serde_json::from_str::<Vec<JsonDataFromPdf>>(&resp.text().await?);
                        match json_response {
                            Ok(json_data) => convert_pdf_json_to_markdown(&json_data),
                            Err(err) => Err(PdfAnalyzerError::SerdeJson(err)),
                        }
                    }
                    Err(error) => Err(PdfAnalyzerError::ErrorPostFile(error)),
                }
            }
            Err(err) => Err(PdfAnalyzerError::ErrorFile(err)),
        }
    }
}

impl Message<AnalyzePdf> for PdfAnalyzer {
    type Response = String;

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, msg: &AnalyzePdf) -> Result<(), PdfAnalyzerError> {
        info!("path {:?}", msg.file_path);
        let markdown = self.post_pdf_analyzer(&msg.file_path).await?;
        ctx.reply(markdown).await?;
        Ok(())
    }
}

impl Actor for PdfAnalyzer {
    type Error = PdfAnalyzerError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("{} Started", ctx.id());

        let mut stream = ctx.recv().await?;

        while let Some(Ok(frame)) = stream.next().await {
            if let Some(input) = frame.is::<AnalyzePdf>() {
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
