use anyhow::{Context, Result};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{mpsc, Mutex},
};
use tracing::debug;

use super::Transport;

pub struct McpServer {
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Clone)]
pub struct StdioTransport {
    process: Arc<Mutex<Option<Child>>>,
    stdin: Arc<Mutex<Option<tokio::process::ChildStdin>>>,
    stdout: Arc<Mutex<Option<tokio::process::ChildStdout>>>,
}

impl StdioTransport {
    pub fn new() -> Self {
        Self {
            process: Arc::new(Mutex::new(None)),
            stdin: Arc::new(Mutex::new(None)),
            stdout: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn start_process(&self, server: &McpServer) -> Result<()> {
        debug!("Starting server process: {} {:?}", server.command, server.args);

        let mut child = Command::new(&server.command)
            .args(&server.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .context("Failed to start MCP server process")?;

        debug!("Server process started successfully");

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();

        *self.stdin.lock().await = stdin;
        *self.stdout.lock().await = stdout;
        *self.process.lock().await = Some(child);

        Ok(())
    }
}

impl Transport for StdioTransport {
    fn start(&mut self, request_tx: mpsc::Sender<String>) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let stdout = self.stdout.clone();

        Box::pin(async move {
            if let Some(stdout) = &mut *stdout.lock().await {
                let mut reader = BufReader::new(stdout);
                let mut line = String::new();

                while reader.read_line(&mut line).await? > 0 {
                    debug!("Received [stdio]: {}", line.trim());
                    if request_tx.send(line.clone()).await.is_err() {
                        debug!("Request channel closed - stopping read loop");
                        break;
                    }
                    line.clear();
                }
                Ok(())
            } else {
                Err(anyhow::anyhow!("Failed to get stdout handle"))
            }
        })
    }

    fn send_message(&mut self, message: String) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let stdin = self.stdin.clone();
        Box::pin(async move {
            if !message.is_empty() {
                if let Some(stdin) = &mut *stdin.lock().await {
                    debug!("Sending [stdio]: {}", message);
                    stdin.write_all(message.as_bytes()).await.context("Failed to write message")?;
                    stdin.write_all(b"\n").await.context("Failed to write newline")?;
                    stdin.flush().await.context("Failed to flush stdin")?;
                }
            }
            Ok(())
        })
    }
}

// WebSocketTransport client implementation will go here...
