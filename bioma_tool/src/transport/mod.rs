pub mod sse;
pub mod stdio;

use anyhow::Result;
use std::net::SocketAddr;
use tokio::sync::mpsc;

pub trait Transport {
    fn start(&mut self, request_tx: mpsc::Sender<String>) -> impl std::future::Future<Output = Result<()>>;
    fn send(&mut self, message: String) -> impl std::future::Future<Output = Result<()>>;
}

#[derive(Clone)]
pub enum TransportType {
    Stdio(stdio::StdioTransport),
    Sse(sse::SseTransport),
}

impl Transport for TransportType {
    async fn start(&mut self, request_tx: mpsc::Sender<String>) -> Result<()> {
        match self {
            TransportType::Stdio(t) => t.start(request_tx).await,
            TransportType::Sse(t) => t.start(request_tx).await,
        }
    }

    async fn send(&mut self, message: String) -> Result<()> {
        match self {
            TransportType::Stdio(t) => t.send(message).await,
            TransportType::Sse(t) => t.send(message).await,
        }
    }
}

/// Create a transport from a specified type
pub fn create_transport(
    transport_type: &str,
    server_config: Option<&crate::client::ServerConfig>,
) -> Result<TransportType> {
    match transport_type.to_lowercase().as_str() {
        "stdio" => {
            if let Some(config) = server_config {
                Ok(TransportType::Stdio(stdio::StdioTransport::new_client(config)?))
            } else {
                Ok(TransportType::Stdio(stdio::StdioTransport::new_server()))
            }
        }
        "sse" => {
            if let Some(config) = server_config {
                Ok(TransportType::Sse(sse::SseTransport::new_client(config)?))
            } else {
                // Default port for SSE server
                let addr: SocketAddr =
                    "[::1]:3000".parse().map_err(|_| anyhow::anyhow!("Invalid default SSE server address"))?;
                Ok(TransportType::Sse(sse::SseTransport::new_server(addr)))
            }
        }
        _ => Err(anyhow::anyhow!("Unsupported transport type: {}", transport_type)),
    }
}
