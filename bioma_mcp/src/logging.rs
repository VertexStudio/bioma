use crate::schema::{LoggingLevel, LoggingMessageNotificationParams};
use crate::transport::TransportSender;
use crate::ConnectionId;
use jsonrpc_core::{Params, Value};
use serde_json::json;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::field::{Field, Visit};
use tracing::{debug, error, trace, warn, Event};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

/// Structure to pass log data through the channel
#[derive(Clone, Debug)]
struct QueuedLog {
    level: LoggingLevel,
    target: String,
    message: String,
    connection_ids: Vec<ConnectionId>,
}

#[derive(Clone)]
pub struct McpLoggingLayer {
    log_sender: mpsc::Sender<QueuedLog>,
    client_levels: Arc<RwLock<HashMap<ConnectionId, LoggingLevel>>>,
}

struct Worker {
    log_receiver: mpsc::Receiver<QueuedLog>,
    transport: Arc<RwLock<TransportSender>>,
    client_levels: Arc<RwLock<HashMap<ConnectionId, LoggingLevel>>>,
}

impl McpLoggingLayer {
    pub fn new(transport: TransportSender) -> Self {
        let (log_sender, log_receiver) = mpsc::channel(1024); // Configurable buffer size
        let client_levels = Arc::new(RwLock::new(HashMap::new()));
        let transport = Arc::new(RwLock::new(transport));

        let worker = Worker { log_receiver, transport, client_levels: Arc::clone(&client_levels) };

        // Spawn the background worker task
        tokio::spawn(worker.run());

        Self { log_sender, client_levels }
    }

    pub async fn update_transport(&self, transport: TransportSender) {
        let transport_arc = Arc::new(RwLock::new(transport));

        // Create a new worker with the updated transport
        let (tx, rx) = mpsc::channel(1024);
        let worker =
            Worker { log_receiver: rx, transport: transport_arc, client_levels: Arc::clone(&self.client_levels) };

        tokio::spawn(worker.run());

        // Replace the sender with the new one
        let mut old_sender = self.log_sender.clone();
        // This effectively replaces self.log_sender with tx
        std::mem::swap(&mut old_sender, &mut tx.clone());
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
}

/// Compares log levels to determine if a message should be sent
/// Returns true if the client level (first param) is "higher" (more verbose)
/// or equal to the message level (second param)
fn compare_log_levels(client_level: &LoggingLevel, message_level: &LoggingLevel) -> bool {
    // For LoggingLevel, lower numerical value means higher severity
    // So Debug > Info > Warning > Error etc.
    match client_level {
        LoggingLevel::Debug => true, // Debug level receives all logs
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

impl Worker {
    async fn run(mut self) {
        debug!("MCP Logging worker task started");

        while let Some(log_entry) = self.log_receiver.recv().await {
            if log_entry.connection_ids.is_empty() {
                // Get all clients that should receive this log
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
                    continue; // No clients interested in this log level
                }

                // Create notification once for all clients
                let notification = self.create_notification(&log_entry);

                // Send to all interested clients
                let transport_guard = self.transport.read().await;
                for conn_id in clients_to_notify {
                    if let Err(e) = transport_guard.send(notification.clone().into(), conn_id.clone()).await {
                        warn!(connection_id = ?conn_id, error = %e, "Failed to send log notification");
                    }
                }
            } else {
                // Specific connection IDs were provided
                // Create notification once for all clients
                let notification = self.create_notification(&log_entry);

                // Send to specified clients
                let transport_guard = self.transport.read().await;
                for conn_id in log_entry.connection_ids {
                    if let Err(e) = transport_guard.send(notification.clone().into(), conn_id.clone()).await {
                        warn!(connection_id = ?conn_id, error = %e, "Failed to send log notification");
                    }
                }
            }
        }

        debug!("MCP Logging worker task stopped");
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
                // Fallback params
                jsonrpc_core::Notification {
                    jsonrpc: Some(jsonrpc_core::Version::V2),
                    method: "notifications/message".to_string(),
                    params: jsonrpc_core::Params::Map(serde_json::Map::new()),
                }
            }
            Err(e) => {
                error!(error = %e, "Failed to serialize log parameters");
                // Fallback params
                jsonrpc_core::Notification {
                    jsonrpc: Some(jsonrpc_core::Version::V2),
                    method: "notifications/message".to_string(),
                    params: jsonrpc_core::Params::Map(serde_json::Map::new()),
                }
            }
        }
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
        tracing::Level::TRACE => LoggingLevel::Debug, // MCP doesn't have a TRACE equivalent
    }
}

impl<S> Layer<S> for McpLoggingLayer
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        // Skip internal tokio events - only process events from our library
        let target = event.metadata().target();
        if !target.starts_with("bioma_mcp") {
            return;
        }

        // Apply early filtering - ignore trace level events globally
        // This prevents filling the queue with unwanted events
        if *event.metadata().level() == tracing::Level::TRACE {
            return;
        }

        // Convert to MCP log level
        let mcp_level = tracing_level_to_mcp_level(event.metadata().level());

        // Extract message using the visitor
        let mut visitor = LogVisitor::new();
        event.record(&mut visitor);

        let log_entry = QueuedLog {
            level: mcp_level,
            target: target.to_string(),
            message: visitor.get_message(),
            connection_ids: Vec::new(), // Empty means "send to all eligible clients"
        };

        // Try to send to the worker task's queue
        match self.log_sender.try_send(log_entry.clone()) {
            Ok(_) => {
                // Successfully queued
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                // Log queue is full - this avoids a feedback loop if we used tracing
                eprintln!("WARNING: MCP Logging queue full. Dropping log message. {:?}, event: {:?}", log_entry, event);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                // Worker task has stopped
                eprintln!("ERROR: MCP Logging channel closed. Worker task may have stopped.");
            }
        }
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
