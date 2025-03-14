pub mod sse;
pub mod stdio;

use crate::JsonRpcMessage;
use anyhow::Result;
use std::future::Future;
use tokio::task::JoinHandle;

pub trait TransportMetadata: Send + Sync {}

pub trait Transport {
    // Start processing messages
    fn start(&mut self) -> impl Future<Output = Result<JoinHandle<Result<()>>>>;

    // Send a JSON-RPC message with optional metadata
    fn send(
        &mut self,
        message: JsonRpcMessage,
        metadata: Option<&dyn TransportMetadata>,
    ) -> impl Future<Output = Result<()>>;

    // Close the connection
    fn close(&mut self) -> impl Future<Output = Result<()>>;
}

#[derive(Clone)]
pub enum TransportType {
    Stdio(stdio::StdioTransport),
    Sse(sse::SseTransport),
}

impl Transport for TransportType {
    async fn start(&mut self) -> Result<JoinHandle<Result<()>>> {
        match self {
            TransportType::Stdio(t) => t.start().await,
            TransportType::Sse(t) => t.start().await,
        }
    }

    async fn send(&mut self, message: JsonRpcMessage, metadata: Option<&dyn TransportMetadata>) -> Result<()> {
        match self {
            TransportType::Stdio(t) => t.send(message, metadata).await,
            TransportType::Sse(t) => t.send(message, metadata).await,
        }
    }

    async fn close(&mut self) -> Result<()> {
        match self {
            TransportType::Stdio(t) => t.close().await,
            TransportType::Sse(t) => t.close().await,
        }
    }
}
