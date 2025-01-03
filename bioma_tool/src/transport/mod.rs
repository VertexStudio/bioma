pub mod server;
pub mod client;

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
    ServerStdio(server::StdioTransport),
    ClientStdio(client::StdioTransport),
    ServerWebSocket(server::WebSocketTransport),
}

impl Transport for TransportType {
    fn start(&mut self, request_tx: mpsc::Sender<String>) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        match self {
            TransportType::ServerStdio(t) => t.start(request_tx),
            TransportType::ClientStdio(t) => t.start(request_tx),
            TransportType::ServerWebSocket(t) => t.start(request_tx),
        }
    }

    fn send_message(&mut self, message: String) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        match self {
            TransportType::ServerStdio(t) => t.send_message(message),
            TransportType::ClientStdio(t) => t.send_message(message),
            TransportType::ServerWebSocket(t) => t.send_message(message),
        }
    }
}
