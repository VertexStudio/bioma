use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

#[derive(Deserialize)]
pub struct JsonDataFromPdf {
    pub text: String,
    #[serde(rename = "type")]
    pub item_type: String,
}

pub fn convert_pdf_json_to_markdown(json_data: &Vec<JsonDataFromPdf>) -> Result<String, ConvertToMarkdownError> {
    let mut markdown = String::new();

    for item in json_data {
        match item.item_type.as_str() {
            "Title" => markdown.push_str(&format!("# {}", item.text)),
            "Section header" => markdown.push_str(&format!("# {}", item.text)),
            "Text" => markdown.push_str(&format!("{}", item.text)),
            "Table" => markdown.push_str(""),
            _ => markdown.push_str(&format!("{}", item.text))
        }
        markdown.push_str("\n");
    }

    Ok(markdown)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertToMarkdown {
    pub file_path: String,
}

#[derive(thiserror::Error, Debug)]
pub enum ConvertToMarkdownError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Error get file")]
    ErrorFile(#[from] std::io::Error),
    #[error("Error post file to pdf-analyzer")]
    ErrorPostFile(#[from] reqwest::Error),
    #[error("Error serde data")]
    SerdeJson(#[from] serde_json::Error),
}

impl ActorError for ConvertToMarkdownError {}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PdfToMarkdown {}

impl PdfToMarkdown {
    async fn post_pdf_analyzer(&self, file_path: &str) -> Result<String, ConvertToMarkdownError> {
        let form_result = reqwest::multipart::Form::new()
            .file("file", file_path).await;

        match form_result { 
            Ok(form) => {
                let response = reqwest::Client::new()
                    .post("http://localhost:5060")
                    .multipart(form)
                    .send()
                    .await;

                match response {
                    Ok(resp) => {
                        let json_response = serde_json::from_str::<Vec<JsonDataFromPdf>>(&resp.text().await?);
                        match json_response {
                            Ok(json_data) => convert_pdf_json_to_markdown(&json_data),
                            Err(err) => Err(ConvertToMarkdownError::SerdeJson(err)),
                        }  
                    },
                    Err(error) => Err(ConvertToMarkdownError::ErrorPostFile(error)),
                }
             },
             Err(err) => Err(ConvertToMarkdownError::ErrorFile(err))
        }
    }
}

impl Message<ConvertToMarkdown> for PdfToMarkdown {
    type Response = String;

    async fn handle(&mut self, _ctx: &mut ActorContext<Self>, msg: &ConvertToMarkdown) -> Result<String, ConvertToMarkdownError> {
        info!("pth {:?}", msg.file_path);
        self.post_pdf_analyzer(&msg.file_path).await
    }
}

impl Actor for PdfToMarkdown {
    type Error = ConvertToMarkdownError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("{} Started", ctx.id());

        let mut stream = ctx.recv().await?; 

        while let Some(Ok(frame)) = stream.next().await {
            if let Some(input) = frame.is::<ConvertToMarkdown>() {
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


# [cfg(test)]
mod tests { 
    use super::*;
    use tokio;
    use tracing::{error, info};

    #[tokio::test]
    async fn test_convert_pdf() -> Result<(), ConvertToMarkdownError> { 
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
        
        tracing_subscriber::fmt().with_env_filter(filter).init();

        let file_path = "/home/user/Downloads/file.pdf".to_string();

        let form_result = reqwest::multipart::Form::new()
            .file("file", file_path).await;

        match form_result { 
            Ok(form) => {
                let response = reqwest::Client::new()
                    .post("http://localhost:5060")
                    .multipart(form)
                    .send()
                    .await;

                match response {
                    Ok(resp) => {
                        let json_response = serde_json::from_str::<Vec<JsonDataFromPdf>>(&resp.text().await?);
                        match json_response {
                            Ok(json_data) => {
                                let convert_result = convert_pdf_json_to_markdown(&json_data);
                                info!("{:?}", convert_result)
                            },
                            Err(err) => error!("{:?}", err),
                        }  
                    },
                    Err(err) => error!("{:?}", err),
                }
             },
             Err(err) => error!("{:?}", err)
        }

        Ok(())
    } 
}