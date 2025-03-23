pub mod sse;
pub mod stdio;
pub mod ws;

use crate::ConnectionId;
use crate::JsonRpcMessage;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::future::Future;
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub conn_id: ConnectionId,
    pub message: JsonRpcMessage,
}

pub trait Transport {
    fn start(&mut self) -> impl Future<Output = Result<JoinHandle<Result<()>>>>;

    fn send(&mut self, message: JsonRpcMessage, conn_id: ConnectionId) -> impl Future<Output = Result<()>>;

    fn close(&mut self) -> impl Future<Output = Result<()>>;

    fn sender(&self) -> TransportSender;
}

pub trait SendMessage {
    fn send(&self, message: JsonRpcMessage, conn_id: ConnectionId) -> impl Future<Output = Result<()>>;
}

#[derive(Clone)]
pub enum TransportSenderType {
    Stdio(stdio::StdioTransportSender),
    Sse(sse::SseTransportSender),
    Ws(ws::WsTransportSender),
}

impl TransportSenderType {
    pub async fn send(&self, message: JsonRpcMessage, conn_id: ConnectionId) -> Result<()> {
        match self {
            Self::Stdio(sender) => sender.send(message, conn_id).await,
            Self::Sse(sender) => sender.send(message, conn_id).await,
            Self::Ws(sender) => sender.send(message, conn_id).await,
        }
    }
}

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

    pub async fn send(&self, message: JsonRpcMessage, conn_id: ConnectionId) -> Result<()> {
        self.inner.send(message, conn_id).await
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

    async fn send(&mut self, message: JsonRpcMessage, conn_id: ConnectionId) -> Result<()> {
        match self {
            TransportType::Stdio(t) => t.send(message, conn_id).await,
            TransportType::Sse(t) => t.send(message, conn_id).await,
            TransportType::Ws(t) => t.send(message, conn_id).await,
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
