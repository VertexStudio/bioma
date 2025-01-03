use anyhow::{Context, Result};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{mpsc, Mutex},
};
use tracing::{debug, error};

use super::Transport;

pub struct McpServer {
    pub command: String,
    pub args: Vec<String>,
}

enum StdioMode {
    Server(Arc<Mutex<tokio::io::Stdout>>),
    Client {
        #[allow(unused)]
        process: Arc<Mutex<Child>>,
        stdin: Arc<Mutex<tokio::process::ChildStdin>>,
        stdout: Arc<Mutex<tokio::process::ChildStdout>>,
    },
}

#[derive(Clone)]
pub struct StdioTransport {
    mode: Arc<StdioMode>,
}

impl StdioTransport {
    pub fn new_server() -> Self {
        Self { mode: Arc::new(StdioMode::Server(Arc::new(Mutex::new(tokio::io::stdout())))) }
    }

    pub fn new_client(server: &McpServer) -> Result<Self> {
        let mut child = Command::new(&server.command)
            .args(&server.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .context("Failed to start MCP server process")?;

        let stdin = child.stdin.take().expect("Failed to get stdin");
        let stdout = child.stdout.take().expect("Failed to get stdout");

        Ok(Self {
            mode: Arc::new(StdioMode::Client {
                process: Arc::new(Mutex::new(child)),
                stdin: Arc::new(Mutex::new(stdin)),
                stdout: Arc::new(Mutex::new(stdout)),
            }),
        })
    }
}

impl Transport for StdioTransport {
    fn start(&mut self, request_tx: mpsc::Sender<String>) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let mode = self.mode.clone();
        Box::pin(async move {
            match &*mode {
                StdioMode::Server(_stdout) => {
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
                }
                StdioMode::Client { stdout, .. } => {
                    let mut stdout = stdout.lock().await;
                    let mut reader = BufReader::new(&mut *stdout);
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
                }
            }
        })
    }

    fn send_message(&mut self, message: String) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let mode = self.mode.clone();
        Box::pin(async move {
            if !message.is_empty() {
                match &*mode {
                    StdioMode::Server(stdout) => {
                        let mut stdout = stdout.lock().await;
                        debug!("Sending [stdio]: {}", message);
                        stdout.write_all(message.as_bytes()).await.context("Failed to write message")?;
                        stdout.write_all(b"\n").await.context("Failed to write newline")?;
                        stdout.flush().await.context("Failed to flush stdout")?;
                    }
                    StdioMode::Client { stdin, .. } => {
                        let mut stdin = stdin.lock().await;
                        debug!("Sending [stdio]: {}", message);
                        stdin.write_all(message.as_bytes()).await.context("Failed to write message")?;
                        stdin.write_all(b"\n").await.context("Failed to write newline")?;
                        stdin.flush().await.context("Failed to flush stdin")?;
                    }
                }
            }
            Ok(())
        })
    }
}
