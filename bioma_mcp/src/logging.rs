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

struct LoggingWorker {
    log_receiver: mpsc::Receiver<LoggingMessageNotificationParams>,
    transport: Arc<RwLock<TransportSender>>,
    client_levels: Arc<RwLock<HashMap<ConnectionId, LoggingLevel>>>,
}

#[derive(Clone)]
pub struct McpLoggingLayer {
    log_sender: Arc<RwLock<mpsc::Sender<LoggingMessageNotificationParams>>>,
    client_levels: Arc<RwLock<HashMap<ConnectionId, LoggingLevel>>>,
    #[allow(unused)]
    worker_handle: Arc<RwLock<Option<JoinHandle<()>>>>,
}

impl McpLoggingLayer {
    pub fn new(transport: TransportSender) -> Self {
        let client_levels = Arc::new(RwLock::new(HashMap::new()));
        let transport = Arc::new(RwLock::new(transport));

        let (log_sender, log_receiver) = mpsc::channel(1024);

        let worker = LoggingWorker {
            log_receiver,
            transport: Arc::clone(&transport),
            client_levels: Arc::clone(&client_levels),
        };

        let worker_handle = tokio::spawn(worker.run());

        Self {
            log_sender: Arc::new(RwLock::new(log_sender)),
            client_levels,
            worker_handle: Arc::new(RwLock::new(Some(worker_handle))),
        }
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

    async fn send_log(&self, params: LoggingMessageNotificationParams) {
        let sender = self.log_sender.read().await;
        match sender.try_send(params.clone()) {
            Ok(_) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {}
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }
}

impl LoggingWorker {
    async fn run(mut self) {
        debug!("MCP Logging worker task started");

        while let Some(params) = self.log_receiver.recv().await {
            eprintln!("MCP Logging worker received log: {:?}", params);
            let clients_to_notify = {
                let client_levels = self.client_levels.read().await;
                client_levels
                    .iter()
                    .filter_map(|(conn_id, client_level)| {
                        if compare_log_levels(client_level, &params.level) {
                            Some(conn_id.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            };

            if clients_to_notify.is_empty() {
                continue;
            }

            let notification = self.create_notification(&params);

            let transport_guard = self.transport.read().await;
            for conn_id in clients_to_notify {
                if let Err(_e) = transport_guard.send(notification.clone().into(), conn_id.clone()).await {}
            }
        }

        debug!("MCP Logging worker task stopped");
    }

    fn create_notification(&self, params: &LoggingMessageNotificationParams) -> jsonrpc_core::Notification {
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

        let mcp_level = tracing_level_to_mcp_level(event.metadata().level());

        let mut visitor = LogVisitor::new();
        event.record(&mut visitor);

        let params = LoggingMessageNotificationParams {
            level: mcp_level,
            logger: Some(target.to_string()),
            data: visitor.get_message().into(),
        };

        let rt = tokio::runtime::Handle::current();

        let this = self.clone();

        rt.spawn(async move {
            this.send_log(params).await;
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
