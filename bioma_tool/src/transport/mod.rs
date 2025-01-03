pub mod stdio;

// Export client types once implemented

use anyhow::Result;
use std::future::Future;
use std::pin::Pin;
use tokio::sync::mpsc;

pub trait Transport {
    fn start(&mut self, request_tx: mpsc::Sender<String>) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;
    fn send_message(&mut self, message: String) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;
}

#[derive(Clone)]
pub enum TransportType {
    Stdio(stdio::StdioTransport),
}

impl Transport for TransportType {
    fn start(&mut self, request_tx: mpsc::Sender<String>) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        match self {
            TransportType::Stdio(t) => t.start(request_tx),
        }
    }

    fn send_message(&mut self, message: String) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        match self {
            TransportType::Stdio(t) => t.send_message(message),
        }
    }
}
