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
}

impl Transport for TransportType {
    fn start(&mut self, request_tx: mpsc::Sender<String>) -> impl std::future::Future<Output = Result<()>> {
        async move {
            match self {
                TransportType::Stdio(t) => t.start(request_tx).await,
            }
        }
    }

    fn send(&mut self, message: String) -> impl std::future::Future<Output = Result<()>> {
        async move {
            match self {
                TransportType::Stdio(t) => t.send(message).await,
            }
        }
    }
}
