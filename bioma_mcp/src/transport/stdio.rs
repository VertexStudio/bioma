use super::{SendMessage, Transport, TransportSender};
use crate::client::StdioConfig;
use crate::transport::Message;
use crate::{ConnectionId, JsonRpcMessage};
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
    Server {
        on_message: mpsc::Sender<Message>,
        stdout: Arc<Mutex<tokio::io::Stdout>>,
    },
    Client {
        on_message: mpsc::Sender<JsonRpcMessage>,

        #[allow(unused)]
        process: Arc<Mutex<Child>>,

        stdin: Arc<Mutex<tokio::process::ChildStdin>>,
        stdout: Arc<Mutex<tokio::process::ChildStdout>>,
    },
}

#[derive(Clone)]
pub struct StdioTransport {
    mode: Arc<StdioMode>,
    #[allow(unused)]
    on_error: mpsc::Sender<Error>,
    #[allow(unused)]
    on_close: mpsc::Sender<()>,
}

#[derive(Clone)]
pub struct StdioTransportSender {
    mode: Arc<StdioMode>,
}

impl SendMessage for StdioTransportSender {
    async fn send(&self, message: JsonRpcMessage, _client_id: ConnectionId) -> Result<()> {
        let json = serde_json::to_string(&message).context("Failed to serialize message")?;
        let message_with_newline = format!("{}\n", json);

        match &*self.mode {
            StdioMode::Server { stdout, .. } => {
                let mut stdout = stdout.lock().await;
                stdout.write_all(message_with_newline.as_bytes()).await.context("Failed to write to stdout")?;
                stdout.flush().await.context("Failed to flush stdout")?;
            }
            StdioMode::Client { stdin, .. } => {
                let mut stdin = stdin.lock().await;
                stdin.write_all(message_with_newline.as_bytes()).await.context("Failed to write to stdin")?;
                stdin.flush().await.context("Failed to flush stdin")?;
            }
        }

        Ok(())
    }
}

impl StdioTransport {
    pub fn new_server(
        on_message: mpsc::Sender<Message>,
        on_error: mpsc::Sender<Error>,
        on_close: mpsc::Sender<()>,
    ) -> Self {
        Self {
            mode: Arc::new(StdioMode::Server { stdout: Arc::new(Mutex::new(tokio::io::stdout())), on_message }),
            on_error,
            on_close,
        }
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
            .envs(config.env.iter())
            .spawn()
            .context("Failed to start MCP server process")?;

        let stdin = child.stdin.take().expect("Failed to get stdin");
        let stdout = child.stdout.take().expect("Failed to get stdout");

        Ok(Self {
            mode: Arc::new(StdioMode::Client {
                on_message,
                process: Arc::new(Mutex::new(child)),
                stdin: Arc::new(Mutex::new(stdin)),
                stdout: Arc::new(Mutex::new(stdout)),
            }),
            on_error,
            on_close,
        })
    }
}

impl Transport for StdioTransport {
    async fn start(&mut self) -> Result<JoinHandle<Result<()>>> {
        let mode = self.mode.clone();
        let handle = tokio::spawn(async move {
            match &*mode {
                StdioMode::Server { on_message, stdout: _stdout } => {
                    let conn_id = ConnectionId::new(None);
                    let stdin = tokio::io::stdin();
                    let mut lines = BufReader::new(stdin).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        debug!("Server received [stdio]: {}", line);
                        let request = match serde_json::from_str::<JsonRpcMessage>(&line) {
                            Ok(request) => request,
                            Err(e) => {
                                error!("Failed to parse message: {}", e);
                                continue;
                            }
                        };
                        let message = Message { message: request, conn_id: conn_id.clone() };
                        if on_message.send(message).await.is_err() {
                            error!("Failed to send request through channel");
                            break;
                        }
                    }
                    Ok(())
                }
                StdioMode::Client { on_message, stdout, .. } => {
                    let mut stdout = stdout.lock().await;
                    let mut lines = BufReader::new(&mut *stdout).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        debug!("Client received [stdio]: {}", line);
                        let request = match serde_json::from_str::<JsonRpcMessage>(&line) {
                            Ok(request) => request,
                            Err(e) => {
                                error!("Failed to parse message: {}", e);
                                continue;
                            }
                        };
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

    async fn send(&mut self, message: JsonRpcMessage, _client_id: ConnectionId) -> Result<()> {
        let message_str = serde_json::to_string(&message)?;
        match &*self.mode {
            StdioMode::Server { stdout, .. } => {
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
                StdioMode::Server { stdout, .. } => {
                    debug!("Closing server stdio transport");
                    let mut stdout = stdout.lock().await;
                    stdout.flush().await.context("Failed to flush stdout on close")?;
                }
                StdioMode::Client { stdin, process, .. } => {
                    debug!("Closing client stdio transport");
                    let mut stdin = stdin.lock().await;
                    stdin.flush().await.context("Failed to flush stdin on close")?;

                    let mut child = process.lock().await;
                    if let Err(e) = child.start_kill() {
                        debug!("Failed to kill child process: {}", e);
                    }
                }
            }
            Ok(())
        }
    }

    fn sender(&self) -> TransportSender {
        TransportSender::new_stdio(StdioTransportSender { mode: self.mode.clone() })
    }
}
