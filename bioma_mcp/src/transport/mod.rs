pub mod sse;
pub mod stdio;
pub mod ws;

use crate::ClientId;
use crate::JsonRpcMessage;
use anyhow::Result;
use std::future::Future;
use tokio::task::JoinHandle;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub client_id: ClientId,
    pub message: JsonRpcMessage,
}

pub trait Transport {
    // Start processing messages
    fn start(&mut self) -> impl Future<Output = Result<JoinHandle<Result<()>>>>;

    // Send a JSON-RPC message with client_id
    fn send(&mut self, message: JsonRpcMessage, client_id: ClientId) -> impl Future<Output = Result<()>>;

    // Close the connection
    fn close(&mut self) -> impl Future<Output = Result<()>>;
}

#[derive(Clone)]
pub enum TransportType {
    Stdio(stdio::StdioTransport),
    Sse(sse::SseTransport),
    Ws(ws::WsTransport),
}

impl Transport for TransportType {
    async fn start(&mut self) -> Result<JoinHandle<Result<()>>> {
        match self {
            TransportType::Stdio(t) => t.start().await,
            TransportType::Sse(t) => t.start().await,
            TransportType::Ws(t) => t.start().await,
        }
    }

    async fn send(&mut self, message: JsonRpcMessage, client_id: ClientId) -> Result<()> {
        match self {
            TransportType::Stdio(t) => t.send(message, client_id).await,
            TransportType::Sse(t) => t.send(message, client_id).await,
            TransportType::Ws(t) => t.send(message, client_id).await,
        }
    }

    async fn close(&mut self) -> Result<()> {
        match self {
            TransportType::Stdio(t) => t.close().await,
            TransportType::Sse(t) => t.close().await,
            TransportType::Ws(t) => t.close().await,
        }
    }
}
