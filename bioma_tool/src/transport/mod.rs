pub mod stdio;

use crate::JsonRpcMessage;
use anyhow::Result;
use tokio::sync::mpsc;

pub trait Transport {
    fn start(&mut self, request_tx: mpsc::Sender<JsonRpcMessage>) -> impl std::future::Future<Output = Result<()>>;
    fn send(&mut self, message: JsonRpcMessage) -> impl std::future::Future<Output = Result<()>>;
}

#[derive(Clone)]
pub enum TransportType {
    Stdio(stdio::StdioTransport),
}

impl Transport for TransportType {
    async fn start(&mut self, request_tx: mpsc::Sender<JsonRpcMessage>) -> Result<()> {
        match self {
            TransportType::Stdio(t) => t.start(request_tx).await,
        }
    }

    async fn send(&mut self, message: JsonRpcMessage) -> Result<()> {
        match self {
            TransportType::Stdio(t) => t.send(message).await,
        }
    }
}
