pub mod stdio;

use crate::JsonRpcMessage;
use anyhow::{Error, Result};
use std::future::Future;
use tokio::task::JoinHandle;

pub trait Transport {
    // Start processing messages
    fn start(&mut self) -> impl Future<Output = Result<JoinHandle<Result<()>>>>;

    // Send a JSON-RPC message
    fn send(&mut self, message: JsonRpcMessage) -> impl Future<Output = Result<()>>;

    // Close the connection
    fn close(&mut self) -> impl Future<Output = Result<()>>;
}

#[derive(Clone)]
pub enum TransportType {
    Stdio(stdio::StdioTransport),
}

impl Transport for TransportType {
    async fn start(&mut self) -> Result<JoinHandle<Result<()>>> {
        match self {
            TransportType::Stdio(t) => t.start().await,
        }
    }

    async fn send(&mut self, message: JsonRpcMessage) -> Result<()> {
        match self {
            TransportType::Stdio(t) => t.send(message).await,
        }
    }

    async fn close(&mut self) -> Result<()> {
        match self {
            TransportType::Stdio(t) => t.close().await,
        }
    }
}
