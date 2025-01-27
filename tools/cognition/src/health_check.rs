use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tracing::{error, info};
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
#[serde(untagged)]
pub enum Responses {
    #[serde(rename = "markitdown")]
    Markitdown { is_healthy: bool },
}

/// Check if posible to create a TCP connection to the provided endpoint
pub async fn check_endpoint(endpoint: Url) -> HealthStatus {
    info!("Checking {}", endpoint);

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

/// Check if posible to create a TCP connection to the provided endpoint
pub async fn check_markitdown(endpoint: Url) -> Responses {
    info!("Checking {}", endpoint);

    let check_endpoint = check_endpoint(endpoint).await;

    Responses::Markitdown { is_healthy: check_endpoint.is_healthy }
}
