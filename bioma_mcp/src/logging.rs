use crate::schema::{LoggingLevel, LoggingMessageNotificationParams};
use crate::transport::TransportSender;
use crate::ConnectionId;
use jsonrpc_core::{Params, Value};
use serde_json::json;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::field::{Field, Visit};
use tracing::{info, Event};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

#[derive(Clone)]
pub struct McpLoggingLayer {
    pub transport: Arc<RwLock<TransportSender>>,
    client_levels: Arc<RwLock<HashMap<ConnectionId, LoggingLevel>>>,
}

impl McpLoggingLayer {
    pub fn new(transport: TransportSender) -> Self {
        Self { transport: Arc::new(RwLock::new(transport)), client_levels: Arc::new(RwLock::new(HashMap::new())) }
    }

    pub async fn update_transport(&self, transport: TransportSender) {
        let mut t = self.transport.write().await;
        *t = transport;
    }

    pub async fn set_level(&self, conn_id: ConnectionId, level: LoggingLevel) {
        let mut levels = self.client_levels.write().await;
        levels.insert(conn_id, level);
    }

    pub async fn remove_client(&self, conn_id: &ConnectionId) {
        let mut levels = self.client_levels.write().await;
        levels.remove(conn_id);
    }

    async fn should_send(&self, conn_id: &ConnectionId, level: &LoggingLevel) -> bool {
        let levels = self.client_levels.read().await;

        if let Some(client_level) = levels.get(conn_id) {
            match client_level {
                LoggingLevel::Emergency => matches!(level, LoggingLevel::Emergency),
                LoggingLevel::Alert => matches!(level, LoggingLevel::Emergency | LoggingLevel::Alert),
                LoggingLevel::Critical => {
                    matches!(level, LoggingLevel::Emergency | LoggingLevel::Alert | LoggingLevel::Critical)
                }
                LoggingLevel::Error => {
                    matches!(
                        level,
                        LoggingLevel::Emergency | LoggingLevel::Alert | LoggingLevel::Critical | LoggingLevel::Error
                    )
                }
                LoggingLevel::Warning => {
                    matches!(
                        level,
                        LoggingLevel::Emergency
                            | LoggingLevel::Alert
                            | LoggingLevel::Critical
                            | LoggingLevel::Error
                            | LoggingLevel::Warning
                    )
                }
                LoggingLevel::Notice => {
                    matches!(
                        level,
                        LoggingLevel::Emergency
                            | LoggingLevel::Alert
                            | LoggingLevel::Critical
                            | LoggingLevel::Error
                            | LoggingLevel::Warning
                            | LoggingLevel::Notice
                    )
                }
                LoggingLevel::Info => {
                    matches!(
                        level,
                        LoggingLevel::Emergency
                            | LoggingLevel::Alert
                            | LoggingLevel::Critical
                            | LoggingLevel::Error
                            | LoggingLevel::Warning
                            | LoggingLevel::Notice
                            | LoggingLevel::Info
                    )
                }
                LoggingLevel::Debug => true,
            }
        } else {
            // If no level is set for this client, use Info as the default level
            matches!(
                level,
                LoggingLevel::Emergency
                    | LoggingLevel::Alert
                    | LoggingLevel::Critical
                    | LoggingLevel::Error
                    | LoggingLevel::Warning
                    | LoggingLevel::Notice
                    | LoggingLevel::Info
            )
        }
    }

    async fn send_log_notification(
        &self,
        conn_id: ConnectionId,
        level: LoggingLevel,
        logger: Option<String>,
        message: String,
    ) {
        // Only send if this log level should be sent to this client
        if !self.should_send(&conn_id, &level).await {
            info!("not sending log notification to client: {:?}", conn_id);
            return;
        }

        let params = LoggingMessageNotificationParams { level, logger, data: message.into() };

        let notification = jsonrpc_core::Notification {
            jsonrpc: Some(jsonrpc_core::Version::V2),
            method: "notifications/message".to_string(),
            params: jsonrpc_core::Params::Map(
                serde_json::to_value(params).unwrap_or_default().as_object().cloned().unwrap_or_default(),
            ),
        };

        let transport = self.transport.read().await;
        let _ = transport.send(notification.into(), conn_id).await;
    }
}

struct LogVisitor {
    fields: HashMap<String, String>,
    message: Option<String>,
}

impl LogVisitor {
    fn new() -> Self {
        Self { fields: HashMap::new(), message: None }
    }
}

impl Visit for LogVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{:?}", value));
        } else {
            self.fields.insert(field.name().to_string(), format!("{:?}", value));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.insert(field.name().to_string(), value.to_string());
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields.insert(field.name().to_string(), value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields.insert(field.name().to_string(), value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields.insert(field.name().to_string(), value.to_string());
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.fields.insert(field.name().to_string(), format!("{}", value));
    }
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
        info!("on_event called with level: {:?}", event.metadata().level());
        let mcp_level = tracing_level_to_mcp_level(event.metadata().level());
        info!("converted to mcp_level: {:?}", mcp_level);

        let mut visitor = LogVisitor::new();
        event.record(&mut visitor);

        let message = visitor.message.unwrap_or_default();
        info!("message: {}", message);

        // Include target module as logger
        let logger = Some(event.metadata().target().to_string());
        info!("logger target: {:?}", logger);

        // Need to send to all connected clients
        let layer = self.clone();

        tokio::spawn(async move {
            let clients: Vec<ConnectionId> = {
                let levels = layer.client_levels.read().await;
                levels.keys().cloned().collect()
            };

            for client_id in clients {
                info!("sending notification to client: {:?}", client_id);
                layer
                    .send_log_notification(client_id.clone(), mcp_level.clone(), logger.clone(), message.clone())
                    .await;
            }
        });
    }
}

pub async fn handle_set_level_request(
    params: Params,
    conn_id: ConnectionId,
    logging_layer: Arc<McpLoggingLayer>,
) -> jsonrpc_core::Result<Value> {
    let params: crate::schema::SetLevelRequestParams = params.parse().map_err(|e| {
        tracing::error!("Failed to parse logging/setLevel parameters: {}", e);
        jsonrpc_core::Error::invalid_params(e.to_string())
    })?;

    logging_layer.set_level(conn_id.clone(), params.level.clone()).await;

    tracing::info!("Client {} set log level to {:?}", conn_id, params.level);

    Ok(json!({}))
}
