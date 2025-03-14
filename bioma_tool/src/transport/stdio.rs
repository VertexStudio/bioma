use super::Transport;
use crate::client::StdioConfig;
use crate::JsonRpcMessage;
use anyhow::{Context, Error, Result};
use std::sync::Arc;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{mpsc, Mutex},
    task::JoinHandle,
};
use tracing::{debug, error};

enum StdioMode {
    Server(Mutex<tokio::io::Stdout>),
    Client {
        // Holds the child process to keep it alive
        #[allow(unused)]
        process: Arc<Mutex<Child>>,
        // Mutexes need to be independent as we need to lock them separately
        stdin: Arc<Mutex<tokio::process::ChildStdin>>,
        stdout: Arc<Mutex<tokio::process::ChildStdout>>,
    },
}

#[derive(Clone)]
pub struct StdioTransport {
    mode: Arc<StdioMode>,
    on_message: mpsc::Sender<JsonRpcMessage>,
    #[allow(unused)]
    on_error: mpsc::Sender<Error>,
    #[allow(unused)]
    on_close: mpsc::Sender<()>,
}

impl StdioTransport {
    pub fn new_server(
        on_message: mpsc::Sender<JsonRpcMessage>,
        on_error: mpsc::Sender<Error>,
        on_close: mpsc::Sender<()>,
    ) -> Self {
        Self { mode: Arc::new(StdioMode::Server(Mutex::new(tokio::io::stdout()))), on_message, on_error, on_close }
    }

    pub async fn new_client(
        config: &StdioConfig,
        on_message: mpsc::Sender<JsonRpcMessage>,
        on_error: mpsc::Sender<Error>,
        on_close: mpsc::Sender<()>,
    ) -> Result<Self> {
        let mut child = Command::new(&config.command)
            .args(&config.args)
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
            on_message,
            on_error,
            on_close,
        })
    }
}

impl Transport for StdioTransport {
    async fn start(&mut self) -> Result<JoinHandle<Result<()>>> {
        // Clone the necessary parts to avoid moving self
        let mode = self.mode.clone();
        let on_message = self.on_message.clone();

        let handle = tokio::spawn(async move {
            match &*mode {
                StdioMode::Server(_stdout) => {
                    let stdin = tokio::io::stdin();
                    let mut lines = BufReader::new(stdin).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        debug!("Server received [stdio]: {}", line);
                        let request = serde_json::from_str::<JsonRpcMessage>(&line)?;
                        if on_message.send(request).await.is_err() {
                            error!("Failed to send request through channel");
                            break;
                        }
                    }
                    Ok(())
                }
                StdioMode::Client { stdout, .. } => {
                    let mut stdout = stdout.lock().await;
                    let mut lines = BufReader::new(&mut *stdout).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        debug!("Client received [stdio]: {}", line);
                        let request = serde_json::from_str::<JsonRpcMessage>(&line)?;
                        if on_message.send(request).await.is_err() {
                            debug!("Request channel closed - stopping read loop");
                            break;
                        }
                    }
                    Ok(())
                }
            }
        });
        Ok(handle)
    }

    async fn send(&mut self, message: JsonRpcMessage, _metadata: serde_json::Value) -> Result<()> {
        let message_str = serde_json::to_string(&message)?;
        match &*self.mode {
            StdioMode::Server(stdout) => {
                debug!("Server sending [stdio]: {}", message_str);
                let mut stdout = stdout.lock().await;
                stdout.write_all(message_str.as_bytes()).await.context("Failed to write message")?;
                stdout.write_all(b"\n").await.context("Failed to write newline")?;
                stdout.flush().await.context("Failed to flush stdout")?;
            }
            StdioMode::Client { stdin, .. } => {
                debug!("Client sending [stdio]: {}", message_str);
                let mut stdin = stdin.lock().await;
                stdin.write_all(message_str.as_bytes()).await.context("Failed to write message")?;
                stdin.write_all(b"\n").await.context("Failed to write newline")?;
                stdin.flush().await.context("Failed to flush stdin")?;
            }
        }
        Ok(())
    }

    fn close(&mut self) -> impl std::future::Future<Output = Result<()>> {
        async move {
            match &*self.mode {
                StdioMode::Server(stdout) => {
                    debug!("Closing server stdio transport");
                    let mut stdout = stdout.lock().await;
                    stdout.flush().await.context("Failed to flush stdout on close")?;
                }
                StdioMode::Client { stdin, process, .. } => {
                    debug!("Closing client stdio transport");
                    let mut stdin = stdin.lock().await;
                    stdin.flush().await.context("Failed to flush stdin on close")?;

                    // Graceful termination of the child process
                    let mut child = process.lock().await;
                    if let Err(e) = child.start_kill() {
                        debug!("Failed to kill child process: {}", e);
                    }
                }
            }
            Ok(())
        }
    }
}
