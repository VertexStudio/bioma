use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::error;
use url::Url;

#[derive(utoipa::ToSchema, thiserror::Error, Serialize, Clone, Debug, Hash, PartialEq, Eq)]
#[serde(untagged)]
pub enum HealthCheckError {
    #[error("Reqwest error: {0}")]
    ReqwestError(String),
    #[error("ParseError: {0}")]
    ParseError(String),
    #[error("OllamaError: {0}")]
    OllamaError(String),
}

#[derive(utoipa::ToSchema, Serialize, Clone, Debug, Hash, PartialEq, Eq)]
pub enum Service {
    #[serde(rename = "surrealdb")]
    SurrealDB,
    #[serde(rename = "ollama_chat")]
    OllamaChat,
    #[serde(rename = "ollama_think")]
    OllamaThink,
    #[serde(rename = "pdf_analyzer")]
    PdfAnalyzer,
    #[serde(rename = "markitdown")]
    Markitdown,
    #[serde(rename = "minio")]
    Minio,
}

#[derive(utoipa::ToSchema, Serialize, Clone, Debug, Hash, PartialEq, Eq)]
pub struct Status {
    pub is_healthy: bool,
    pub error: Option<HealthCheckError>,
}

impl Status {
    fn healthy() -> Self {
        Status { is_healthy: true, error: None }
    }

    fn unhealthy(error: HealthCheckError) -> Self {
        Status { is_healthy: false, error: Some(error) }
    }
}

#[derive(utoipa::ToSchema, Serialize, Clone, Debug, Hash, PartialEq, Eq)]
#[serde(untagged)]
#[schema(example = json!({
    "type": "surrealdb",
    "status": {
        "is_healthy": true,
        "error": null
    }
}))]
#[schema(example = json!({
    "type": "ollama",
    "status": {
        "is_healthy": true,
        "error": null
    },
    "health": {
        "models": [
            {
                "size_vram": 12345,
                "model": "llama2"
            }
        ]
    }
}))]
pub enum Responses {
    #[schema(title = "SurrealDB Response")]
    SurrealDb {
        #[serde(flatten)]
        #[schema(inline)]
        status: Status,
    },
    #[schema(title = "Ollama Response")]
    Ollama {
        #[serde(flatten)]
        #[schema(inline)]
        status: Status,
        health: Option<OllamaHealth>,
    },
    #[schema(title = "PDF Analyzer Response")]
    PdfAnalyzer {
        #[serde(flatten)]
        #[schema(inline)]
        status: Status,
        health: Option<PdfAnalyzerHealth>,
    },
    #[schema(title = "Markitdown Response")]
    Markitdown {
        #[serde(flatten)]
        #[schema(inline)]
        status: Status,
    },
    #[schema(title = "Minio Response")]
    Minio {
        #[serde(flatten)]
        #[schema(inline)]
        status: Status,
    },
}

#[derive(utoipa::ToSchema, Serialize, Deserialize, Clone, Debug, Hash, PartialEq, Eq)]
pub struct OllamaRunningModel {
    size_vram: u64,
    model: String,
}

#[derive(utoipa::ToSchema, Serialize, Deserialize, Clone, Debug, Hash, PartialEq, Eq)]
pub struct OllamaHealth {
    models: Vec<OllamaRunningModel>,
}

#[derive(utoipa::ToSchema, Serialize, Deserialize, Clone, Debug, Hash, PartialEq, Eq)]
pub struct PdfAnalyzerHealth {
    info: String,
}

pub async fn check_markitdown(endpoint: Url) -> Responses {
    let endpoint = endpoint.clone().join("health").unwrap();

    // Create a reqwest client with a timeout
    let client = Client::builder()
        .timeout(Duration::from_secs(10)) // Set the timeout duration
        .build()
        .map_err(|e| HealthCheckError::ReqwestError(e.to_string()));

    let client = match client {
        Ok(client) => client,
        Err(e) => return Responses::Markitdown { status: Status::unhealthy(e) },
    };

    // Make the request using the client
    let response = client.get(endpoint).send().await.map_err(|e| HealthCheckError::ReqwestError(e.to_string()));

    match response {
        Ok(_) => Responses::Markitdown { status: Status::healthy() },
        Err(e) => Responses::Markitdown { status: Status::unhealthy(e) },
    }
}

pub async fn check_ollama(endpoint: Url) -> Responses {
    let endpoint = endpoint.clone().join("api/ps").unwrap();

    let client = Client::builder()
        .timeout(Duration::from_secs(10)) // Set the timeout duration
        .build()
        .map_err(|e| HealthCheckError::ReqwestError(e.to_string()));

    let client = match client {
        Ok(client) => client,
        Err(e) => return Responses::Ollama { status: Status::unhealthy(e), health: None },
    };

    let response = client.get(endpoint).send().await.map_err(|e| HealthCheckError::ReqwestError(e.to_string()));

    

    match response {
        Ok(response) => {
            let response = response.json::<OllamaHealth>().await;

            match response {
                Ok(health) => {

                    if health.models.iter().any(|model| model.size_vram == 0) {
                        return Responses::Ollama {
                            status: Status::unhealthy(HealthCheckError::OllamaError(
                                "Ollama is running on CPU".to_string(),
                            )),
                            health: None,
                        };
                    }

                    Responses::Ollama { status: Status::healthy(), health: Some(health) }
                }
                Err(e) => {
                    error!("Error parsing response: {}", e);
                    Responses::Ollama {
                        status: Status::unhealthy(HealthCheckError::ParseError(e.to_string())),
                        health: None,
                    }
                }
            }
        }
        Err(e) => Responses::Ollama { status: Status::unhealthy(e), health: None },
    }
}

pub async fn check_minio(endpoint: Url) -> Responses {
    let endpoint = endpoint.clone().join("minio/health/live").unwrap();

    // Create a reqwest client with a timeout
    let client = Client::builder()
        .timeout(Duration::from_secs(10)) // Set the timeout duration
        .build()
        .map_err(|e| HealthCheckError::ReqwestError(e.to_string()));

    let client = match client {
        Ok(client) => client,
        Err(e) => return Responses::Ollama { status: Status::unhealthy(e), health: None },
    };

    // Make the request using the client
    let response = client.get(endpoint).send().await.map_err(|e| HealthCheckError::ReqwestError(e.to_string()));

    match response {
        Ok(_) => Responses::Minio { status: Status::healthy() },
        Err(e) => Responses::Minio { status: Status::unhealthy(e) },
    }
}

pub async fn check_pdf_analyzer(endpoint: Url) -> Responses {
    let endpoint = endpoint.clone();

    // Create a reqwest client with a timeout
    let client = Client::builder()
        .timeout(Duration::from_secs(10)) // Set the timeout duration
        .build()
        .map_err(|e| HealthCheckError::ReqwestError(e.to_string()))
        .map_err(|e| HealthCheckError::ReqwestError(e.to_string()));

    let client = match client {
        Ok(client) => client,
        Err(e) => return Responses::PdfAnalyzer { status: Status::unhealthy(e), health: None },
    };

    // Make the request using the client
    let response = client.get(endpoint).send().await.map_err(|e| HealthCheckError::ReqwestError(e.to_string()));

    

    match response {
        Ok(response) => {
            let response = response.text().await;

            

            match response {
                Ok(info) => {
                    Responses::PdfAnalyzer { status: Status::healthy(), health: Some(PdfAnalyzerHealth { info }) }
                }
                Err(e) => Responses::PdfAnalyzer {
                    status: Status::unhealthy(HealthCheckError::ParseError(e.to_string())),
                    health: None,
                },
            }
        }
        Err(e) => Responses::PdfAnalyzer { status: Status::unhealthy(e), health: None },
    }
}

pub async fn check_surrealdb(endpoint: String) -> Responses {
    let endpoint = endpoint.replace("ws", "http");
    let endpoint = Url::parse(&endpoint).and_then(|url| url.join("health"));

    let endpoint = match endpoint {
        Ok(endpoint) => endpoint,
        Err(e) => {
            return Responses::SurrealDb { status: Status::unhealthy(HealthCheckError::ParseError(e.to_string())) }
        }
    };

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| HealthCheckError::ReqwestError(e.to_string()));

    let client = match client {
        Ok(client) => client,
        Err(e) => return Responses::SurrealDb { status: Status::unhealthy(e) },
    };

    let response = client.get(endpoint).send().await.map_err(|e| HealthCheckError::ReqwestError(e.to_string()));

    match response {
        Ok(_) => Responses::SurrealDb { status: Status::healthy() },
        Err(e) => Responses::SurrealDb { status: Status::unhealthy(e) },
    }
}
