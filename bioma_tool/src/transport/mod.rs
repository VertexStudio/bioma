pub mod sse;
pub mod stdio;

use anyhow::Result;
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
