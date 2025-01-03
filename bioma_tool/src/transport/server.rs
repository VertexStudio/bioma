use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::{mpsc, Mutex},
};
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};
use tracing::{debug, error};

use super::Transport;

type WsStream = WebSocketStream<tokio::net::TcpStream>;
type WsWriter = futures::stream::SplitSink<WsStream, Message>;

#[derive(Clone)]
pub struct StdioTransport {
    stdout: Arc<Mutex<tokio::io::Stdout>>,
}

impl StdioTransport {
    pub fn new() -> Self {
        Self { stdout: Arc::new(Mutex::new(tokio::io::stdout())) }
    }
}

impl Transport for StdioTransport {
    fn start(&mut self, request_tx: mpsc::Sender<String>) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move {
            let stdin = tokio::io::stdin();
            let mut lines = BufReader::new(stdin).lines();

            while let Ok(Some(line)) = lines.next_line().await {
                debug!("Received [stdio]: {}", line);
                if request_tx.send(line).await.is_err() {
                    error!("Failed to send request through channel");
                    break;
                }
            }
            Ok(())
        })
    }

    fn send_message(&mut self, message: String) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let stdout = self.stdout.clone();
        Box::pin(async move {
            if !message.is_empty() {
                debug!("Sending [stdio]: {}", message);
                let mut stdout = stdout.lock().await;
                stdout.write_all(message.as_bytes()).await.context("Failed to write message")?;
                stdout.write_all(b"\n").await.context("Failed to write newline")?;
                stdout.flush().await.context("Failed to flush stdout")?;
            }
            Ok(())
        })
    }
}

#[derive(Clone)]
pub struct WebSocketTransport {
    addr: String,
    writer: Arc<Mutex<Option<WsWriter>>>,
}

impl WebSocketTransport {
    pub fn new(addr: String) -> Self {
        Self { addr, writer: Arc::new(Mutex::new(None)) }
    }
}

impl Transport for WebSocketTransport {
    fn start(&mut self, request_tx: mpsc::Sender<String>) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let addr = self.addr.clone();
        let writer = self.writer.clone();

        Box::pin(async move {
            let listener = TcpListener::bind(&addr).await.context("Failed to bind to address")?;
            debug!("WebSocket server listening on: {}", addr);

            while let Ok((stream, _)) = listener.accept().await {
                debug!("New WebSocket connection");
                let ws_stream = accept_async(stream).await.context("Failed to accept WebSocket connection")?;

                let (ws_writer, mut ws_reader) = ws_stream.split();
                *writer.lock().await = Some(ws_writer);

                while let Some(msg) = ws_reader.next().await {
                    match msg {
                        Ok(Message::Text(text)) => {
                            debug!("Received [websocket]: {}", text);
                            if request_tx.send(text.to_string()).await.is_err() {
                                error!("Failed to send request through channel");
                                break;
                            }
                        }
                        Ok(Message::Close(_)) => {
                            debug!("WebSocket connection closed");
                            *writer.lock().await = None;
                            break;
                        }
                        Err(e) => {
                            error!("WebSocket error: {}", e);
                            *writer.lock().await = None;
                            break;
                        }
                        _ => continue,
                    }
                }
            }
            Ok(())
        })
    }

    fn send_message(&mut self, message: String) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let writer = self.writer.clone();
        Box::pin(async move {
            if !message.is_empty() {
                if let Some(writer) = &mut *writer.lock().await {
                    debug!("Sending [websocket]: {}", message);
                    writer.send(Message::Text(message.into())).await.context("Failed to send WebSocket message")?;
                }
            }
            Ok(())
        })
    }
} 