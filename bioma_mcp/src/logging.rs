use crate::schema::{LoggingLevel, LoggingMessageNotificationParams};
use crate::transport::TransportSender;
use crate::ConnectionId;
use jsonrpc_core::{Params, Value};
use serde_json::json;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;
use tracing::field::{Field, Visit};
use tracing::{debug, error, Event};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

#[derive(Clone, Debug)]
struct QueuedLog {
    level: LoggingLevel,
    target: String,
    message: String,
    connection_ids: Vec<ConnectionId>,
}

struct LoggingWorker {
    log_receiver: mpsc::Receiver<QueuedLog>,
    transport: Arc<RwLock<TransportSender>>,
    client_levels: Arc<RwLock<HashMap<ConnectionId, LoggingLevel>>>,
    shutdown_signal: mpsc::Receiver<()>,
}

#[derive(Clone)]
pub struct McpLoggingLayer {
    worker_handle: Arc<RwLock<Option<JoinHandle<()>>>>,
    shutdown_sender: Arc<RwLock<Option<mpsc::Sender<()>>>>,

    log_sender: Arc<RwLock<mpsc::Sender<QueuedLog>>>,

    client_levels: Arc<RwLock<HashMap<ConnectionId, LoggingLevel>>>,
    transport: Arc<RwLock<TransportSender>>,
}

impl McpLoggingLayer {
    pub fn new(transport: TransportSender) -> Self {
        let client_levels = Arc::new(RwLock::new(HashMap::new()));
        let transport = Arc::new(RwLock::new(transport));

        let (log_sender, log_receiver) = mpsc::channel(1024);
        let (shutdown_sender, shutdown_receiver) = mpsc::channel(1);

        let worker = LoggingWorker {
            log_receiver,
            transport: Arc::clone(&transport),
            client_levels: Arc::clone(&client_levels),
            shutdown_signal: shutdown_receiver,
        };

        let worker_handle = tokio::spawn(worker.run());

        Self {
            worker_handle: Arc::new(RwLock::new(Some(worker_handle))),
            shutdown_sender: Arc::new(RwLock::new(Some(shutdown_sender))),
            log_sender: Arc::new(RwLock::new(log_sender)),
            client_levels,
            transport,
        }
    }

    pub async fn update_transport(&self, new_transport: TransportSender) {
        debug!("Updating logging transport");

        self.stop_worker().await;

        {
            let mut transport = self.transport.write().await;
            *transport = new_transport;
        }

        self.start_worker().await;

        debug!("Logging transport updated successfully");
    }

    async fn stop_worker(&self) {
        if let Some(sender) = self.shutdown_sender.write().await.take() {
            let _ = sender.send(()).await;
            debug!("Sent shutdown signal to logging worker");
        }

        if let Some(handle) = self.worker_handle.write().await.take() {
            handle.abort();
            debug!("Aborted previous logging worker");
        }
    }

    async fn start_worker(&self) {
        let (log_sender, log_receiver) = mpsc::channel(1024);
        let (shutdown_sender, shutdown_receiver) = mpsc::channel(1);

        let worker = LoggingWorker {
            log_receiver,
            transport: Arc::clone(&self.transport),
            client_levels: Arc::clone(&self.client_levels),
            shutdown_signal: shutdown_receiver,
        };

        {
            let mut sender = self.log_sender.write().await;
            *sender = log_sender;
        }

        {
            let mut shutdown = self.shutdown_sender.write().await;
            *shutdown = Some(shutdown_sender);
        }

        let worker_handle = tokio::spawn(worker.run());

        {
            let mut handle = self.worker_handle.write().await;
            *handle = Some(worker_handle);
        }

        debug!("Started new logging worker");
    }

    pub async fn set_level(&self, conn_id: ConnectionId, level: LoggingLevel) {
        let mut levels = self.client_levels.write().await;
        debug!(connection_id = ?conn_id, ?level, "Setting logging level for client");
        levels.insert(conn_id, level);
    }

    pub async fn remove_client(&self, conn_id: &ConnectionId) {
        let mut levels = self.client_levels.write().await;
        if levels.remove(conn_id).is_some() {
            debug!(connection_id = ?conn_id, "Removed client from logging");
        }
    }

    async fn send_log(&self, log_entry: QueuedLog) {
        let sender = self.log_sender.read().await;
        match sender.try_send(log_entry.clone()) {
            Ok(_) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                eprintln!("WARNING: MCP Logging queue full. Dropping log message: {:?}", log_entry);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                eprintln!("ERROR: MCP Logging channel closed. Worker task may have stopped.");
            }
        }
    }
}

impl LoggingWorker {
    async fn run(mut self) {
        debug!("MCP Logging worker task started");

        loop {
            tokio::select! {

                Some(log_entry) = self.log_receiver.recv() => {
                    self.process_log(log_entry).await;
                }


                _ = self.shutdown_signal.recv() => {
                    debug!("MCP Logging worker received shutdown signal");
                    break;
                }
            }
        }

        debug!("MCP Logging worker task stopped");
    }

    async fn process_log(&self, log_entry: QueuedLog) {
        if log_entry.connection_ids.is_empty() {
            let clients_to_notify = {
                let client_levels = self.client_levels.read().await;
                client_levels
                    .iter()
                    .filter_map(|(conn_id, client_level)| {
                        if compare_log_levels(client_level, &log_entry.level) {
                            Some(conn_id.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            };

            if clients_to_notify.is_empty() {
                return;
            }

            let notification = self.create_notification(&log_entry);

            let transport_guard = self.transport.read().await;
            for conn_id in clients_to_notify {
                if let Err(_e) = transport_guard.send(notification.clone().into(), conn_id.clone()).await {
                    // Can't log here because we're in the logging worker
                }
            }
        } else {
            let notification = self.create_notification(&log_entry);

            let transport_guard = self.transport.read().await;
            for conn_id in log_entry.connection_ids {
                if let Err(_e) = transport_guard.send(notification.clone().into(), conn_id.clone()).await {
                    // Can't log here because we're in the logging worker
                }
            }
        }
    }

    fn create_notification(&self, log_entry: &QueuedLog) -> jsonrpc_core::Notification {
        let params = LoggingMessageNotificationParams {
            level: log_entry.level.clone(),
            logger: Some(log_entry.target.clone()),
            data: log_entry.message.clone().into(),
        };

        match serde_json::to_value(&params) {
            Ok(Value::Object(map)) => jsonrpc_core::Notification {
                jsonrpc: Some(jsonrpc_core::Version::V2),
                method: "notifications/message".to_string(),
                params: jsonrpc_core::Params::Map(map),
            },
            Ok(_) => {
                error!("Failed to serialize log parameters to JSON map");

                jsonrpc_core::Notification {
                    jsonrpc: Some(jsonrpc_core::Version::V2),
                    method: "notifications/message".to_string(),
                    params: jsonrpc_core::Params::Map(serde_json::Map::new()),
                }
            }
            Err(e) => {
                error!(error = %e, "Failed to serialize log parameters");

                jsonrpc_core::Notification {
                    jsonrpc: Some(jsonrpc_core::Version::V2),
                    method: "notifications/message".to_string(),
                    params: jsonrpc_core::Params::Map(serde_json::Map::new()),
                }
            }
        }
    }
}

fn compare_log_levels(client_level: &LoggingLevel, message_level: &LoggingLevel) -> bool {
    match client_level {
        LoggingLevel::Debug => true,
        LoggingLevel::Info => message_level != &LoggingLevel::Debug,
        LoggingLevel::Notice => !matches!(message_level, LoggingLevel::Debug | LoggingLevel::Info),
        LoggingLevel::Warning => {
            !matches!(message_level, LoggingLevel::Debug | LoggingLevel::Info | LoggingLevel::Notice)
        }
        LoggingLevel::Error => matches!(
            message_level,
            LoggingLevel::Error | LoggingLevel::Critical | LoggingLevel::Alert | LoggingLevel::Emergency
        ),
        LoggingLevel::Critical => {
            matches!(message_level, LoggingLevel::Critical | LoggingLevel::Alert | LoggingLevel::Emergency)
        }
        LoggingLevel::Alert => matches!(message_level, LoggingLevel::Alert | LoggingLevel::Emergency),
        LoggingLevel::Emergency => message_level == &LoggingLevel::Emergency,
    }
}

struct LogVisitor {
    message: Option<String>,
}

impl LogVisitor {
    fn new() -> Self {
        Self { message: None }
    }

    fn get_message(self) -> String {
        self.message.unwrap_or_default()
    }
}

impl Visit for LogVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{:?}", value));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        }
    }

    fn record_i64(&mut self, _field: &Field, _value: i64) {}
    fn record_u64(&mut self, _field: &Field, _value: u64) {}
    fn record_bool(&mut self, _field: &Field, _value: bool) {}
    fn record_error(&mut self, _field: &Field, _value: &(dyn std::error::Error + 'static)) {}
}

fn tracing_level_to_mcp_level(level: &tracing::Level) -> LoggingLevel {
    match *level {
        tracing::Level::ERROR => LoggingLevel::Error,
        tracing::Level::WARN => LoggingLevel::Warning,
        tracing::Level::INFO => LoggingLevel::Info,
        tracing::Level::DEBUG => LoggingLevel::Debug,
        tracing::Level::TRACE => LoggingLevel::Debug,
    }
}

impl<S> Layer<S> for McpLoggingLayer
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let target = event.metadata().target();
        if !target.starts_with("bioma_mcp") {
            return;
        }

        eprintln!("Event: {:?}", event);

        if *event.metadata().level() == tracing::Level::TRACE {
            return;
        }

        let mcp_level = tracing_level_to_mcp_level(event.metadata().level());

        let mut visitor = LogVisitor::new();
        event.record(&mut visitor);

        let log_entry = QueuedLog {
            level: mcp_level,
            target: target.to_string(),
            message: visitor.get_message(),
            connection_ids: Vec::new(),
        };

        let rt = tokio::runtime::Handle::current();

        let this = self.clone();

        rt.spawn(async move {
            this.send_log(log_entry).await;
        });
    }
}

pub async fn handle_set_level_request(
    params: Params,
    conn_id: ConnectionId,
    logging_layer: Arc<McpLoggingLayer>,
) -> jsonrpc_core::Result<Value> {
    let params: crate::schema::SetLevelRequestParams = params.parse().map_err(|e| {
        error!("Failed to parse logging/setLevel parameters: {}", e);
        jsonrpc_core::Error::invalid_params(e.to_string())
    })?;

    logging_layer.set_level(conn_id.clone(), params.level).await;

    Ok(json!({}))
}
