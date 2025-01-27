use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tracing::error;
use url::Url;

#[derive(Serialize, Deserialize, Clone, Debug, Hash, PartialEq, Eq)]
pub struct HealthStatus {
    pub is_healthy: bool,
    // timestamp: String,
}

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
pub struct OllamaHealth {
    version: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Hash, PartialEq, Eq)]
#[serde(untagged)]
pub enum Responses {
    Markitdown { is_healthy: bool },
    Ollama { is_healthy: bool, health: Option<OllamaHealth> },
}

/// Check if posible to create a TCP connection to the provided endpoint
pub async fn check_endpoint(endpoint: &Url) -> HealthStatus {
    let host = match endpoint.host_str() {
        Some(host) => host,
        None => {
            error!("Invalid host");
            return HealthStatus { is_healthy: false };
        }
    };

    let port = match endpoint.port() {
        Some(port) => port,
        None => {
            error!("Invalid port");
            return HealthStatus { is_healthy: false };
        }
    };

    let endpoint = format!("{}:{}", host, port);

    let is_healthy = match TcpStream::connect(&endpoint).await {
        Ok(_) => true,
        Err(_) => false,
    };

    HealthStatus { is_healthy }
}

pub async fn check_markitdown(endpoint: Url) -> Responses {
    let check_endpoint = check_endpoint(&endpoint).await;

    Responses::Markitdown { is_healthy: check_endpoint.is_healthy }
}

pub async fn check_ollama(endpoint: Url) -> Result<Responses, reqwest::Error> {
    let endpoint = endpoint.clone().join("api/version").unwrap();

    let check_endpoint = check_endpoint(&endpoint).await;

    // Create a reqwest client with a timeout
    let client = Client::builder()
        .timeout(Duration::from_secs(10)) // Set the timeout duration
        .build()?;

    // Make the request using the client
    let response = client.get(endpoint).send().await;

    match response {
        Ok(response) => {
            let health = response.json::<OllamaHealth>().await;

            let health = match health {
                Ok(health) => Some(health),
                Err(_) => None,
            };

            return Ok(Responses::Ollama { is_healthy: check_endpoint.is_healthy, health });
        }
        Err(_) => return Ok(Responses::Ollama { is_healthy: check_endpoint.is_healthy, health: None }),
    };
}
