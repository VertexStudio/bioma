pub mod sse;
pub mod stdio;
pub mod ws;

use crate::ClientId;
use crate::JsonRpcMessage;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::future::Future;
use tokio::task::JoinHandle;

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

    // Create a sender that can be used to send messages without locking the transport
    fn sender(&self) -> TransportSender;
}

// A trait for sending messages without locking the transport
pub trait SendMessage {
    fn send(&self, message: JsonRpcMessage, client_id: ClientId) -> impl Future<Output = Result<()>>;
}

#[derive(Clone)]
pub enum TransportSenderType {
    Stdio(stdio::StdioTransportSender),
    Sse(sse::SseTransportSender),
    Ws(ws::WsTransportSender),
}

impl TransportSenderType {
    pub async fn send(&self, message: JsonRpcMessage, client_id: ClientId) -> Result<()> {
        match self {
            Self::Stdio(sender) => sender.send(message, client_id).await,
            Self::Sse(sender) => sender.send(message, client_id).await,
            Self::Ws(sender) => sender.send(message, client_id).await,
        }
    }
}

// A sender that can be cloned and used to send messages without locking the transport
#[derive(Clone)]
pub struct TransportSender {
    inner: TransportSenderType,
}

impl TransportSender {
    pub fn new_stdio(sender: stdio::StdioTransportSender) -> Self {
        Self { inner: TransportSenderType::Stdio(sender) }
    }

    pub fn new_sse(sender: sse::SseTransportSender) -> Self {
        Self { inner: TransportSenderType::Sse(sender) }
    }

    pub fn new_ws(sender: ws::WsTransportSender) -> Self {
        Self { inner: TransportSenderType::Ws(sender) }
    }

    pub async fn send(&self, message: JsonRpcMessage, client_id: ClientId) -> Result<()> {
        self.inner.send(message, client_id).await
    }
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

    fn sender(&self) -> TransportSender {
        match self {
            TransportType::Stdio(t) => t.sender(),
            TransportType::Sse(t) => t.sender(),
            TransportType::Ws(t) => t.sender(),
        }
    }
}
