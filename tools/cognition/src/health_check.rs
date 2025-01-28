use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Serialize, Deserialize, Clone, Debug, Hash, PartialEq, Eq)]
pub enum Service {
    #[serde(rename = "surrealdb")]
    SurrealDB,
    #[serde(rename = "ollama")]
    Ollama,
    #[serde(rename = "pdf_analyzer")]
    PdfAnalyzer,
    #[serde(rename = "markitdown")]
    Markitdown,
    #[serde(rename = "minio")]
    Minio,
}

#[derive(Serialize, Deserialize, Clone, Debug, Hash, PartialEq, Eq)]
#[serde(untagged)]
pub enum Responses {
    SurrealDb { is_healthy: bool },
    Ollama { is_healthy: bool, health: Option<OllamaHealth> },
    PdfAnalyzer { is_healthy: bool, health: Option<PdfAnalyzerHealth> },
    Markitdown { is_healthy: bool },
    Minio { is_healthy: bool },
}

#[derive(Serialize, Deserialize, Clone, Debug, Hash, PartialEq, Eq)]
pub struct OllamaHealth {
    version: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Hash, PartialEq, Eq)]
pub struct PdfAnalyzerHealth {
    info: String,
}

pub async fn check_markitdown(endpoint: Url) -> Result<Responses, reqwest::Error> {
    let endpoint = endpoint.clone().join("health").unwrap();

    // Create a reqwest client with a timeout
    let client = Client::builder()
        .timeout(Duration::from_secs(10)) // Set the timeout duration
        .build()?;

    // Make the request using the client
    let response = client.get(endpoint).send().await;

    let is_healthy = match response {
        Ok(_) => true,
        Err(_) => false,
    };

    Ok(Responses::Markitdown { is_healthy })
}

pub async fn check_ollama(endpoint: Url) -> Result<Responses, reqwest::Error> {
    let endpoint = endpoint.clone().join("api/version").unwrap();

    // Create a reqwest client with a timeout
    let client = Client::builder()
        .timeout(Duration::from_secs(10)) // Set the timeout duration
        .build()?;

    // Make the request using the client
    let response = client.get(endpoint).send().await;

    let health = match response {
        Ok(response) => {
            let response = response.json::<OllamaHealth>().await;

            let health = match response {
                Ok(health) => Responses::Ollama { is_healthy: true, health: Some(health) },
                Err(_) => Responses::Ollama { is_healthy: false, health: None },
            };

            health
        }
        Err(_) => Responses::Ollama { is_healthy: false, health: None },
    };

    Ok(health)
}

pub async fn check_minio(endpoint: Url) -> Result<Responses, reqwest::Error> {
    let endpoint = endpoint.clone().join("minio/health/live").unwrap();

    // Create a reqwest client with a timeout
    let client = Client::builder()
        .timeout(Duration::from_secs(10)) // Set the timeout duration
        .build()?;

    // Make the request using the client
    let response = client.get(endpoint).send().await;

    let is_healthy = match response {
        Ok(_) => true,
        Err(_) => false,
    };

    return Ok(Responses::Minio { is_healthy });
}

pub async fn check_pdf_analyzer(endpoint: Url) -> Result<Responses, reqwest::Error> {
    let endpoint = endpoint.clone();

    // Create a reqwest client with a timeout
    let client = Client::builder()
        .timeout(Duration::from_secs(10)) // Set the timeout duration
        .build()?;

    // Make the request using the client
    let response = client.get(endpoint).send().await;

    let health = match response {
        Ok(response) => {
            let response = response.text().await;

            let health = match response {
                Ok(info) => Responses::PdfAnalyzer { is_healthy: true, health: Some(PdfAnalyzerHealth { info }) },
                Err(_) => Responses::PdfAnalyzer { is_healthy: false, health: None },
            };

            health
        }
        Err(_) => Responses::PdfAnalyzer { is_healthy: false, health: None },
    };

    return Ok(health);
}

pub async fn check_surrealdb(endpoint: String) -> Result<Responses, reqwest::Error> {
    let endpoint = endpoint.replace("ws", "http");
    let endpoint = Url::parse(&endpoint).and_then(|url| url.join("health"));

    let endpoint = match endpoint {
        Ok(endpoint) => endpoint,
        Err(_) => return Ok(Responses::SurrealDb { is_healthy: false }),
    };

    // Create a reqwest client with a timeout
    let client = Client::builder()
        .timeout(Duration::from_secs(10)) // Set the timeout duration
        .build()?;

    // Make the request using the client
    let response = client.get(endpoint).send().await;

    let is_healthy = match response {
        Ok(_) => true,
        Err(_) => false,
    };

    return Ok(Responses::SurrealDb { is_healthy });
}
