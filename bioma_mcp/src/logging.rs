use crate::schema::{LoggingLevel, LoggingMessageNotificationParams};
use crate::transport::TransportSender;
use crate::ConnectionId;
use dashmap::DashMap;
use jsonrpc_core::{Notification, Params, Value};
use std::fmt;
use std::sync::Arc;
use thiserror::Error;
use tracing::field::{Field, Visit};
use tracing::{debug, error, Event};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

#[derive(Debug, Error)]
pub enum LoggingError {
    #[error("Transport error: {0}")]
    Transport(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),
}

#[derive(Clone)]
pub struct McpLoggingLayer {
    target_prefix: String,
    clients: Arc<DashMap<ConnectionId, LoggingLevel>>,
    transport: Arc<TransportSender>,
}

impl McpLoggingLayer {
    pub fn new(transport: TransportSender) -> Self {
        Self::with_target_prefix(transport, "bioma_mcp")
    }

    pub fn with_target_prefix(transport: TransportSender, target_prefix: impl Into<String>) -> Self {
        Self { target_prefix: target_prefix.into(), clients: Arc::new(DashMap::new()), transport: Arc::new(transport) }
    }

    pub fn set_client_level(&self, client_id: ConnectionId, level: LoggingLevel) {
        debug!(client_id = ?client_id, level = ?level, "Setting client log level");
        self.clients.insert(client_id, level);
    }

    pub fn remove_client(&self, client_id: &ConnectionId) {
        if self.clients.remove(client_id).is_some() {
            debug!(client_id = ?client_id, "Removed client from logging");
        }
    }

    async fn send_log(&self, params: LoggingMessageNotificationParams) -> Result<(), LoggingError> {
        let eligible_clients = self.get_eligible_clients(&params.level);
        if eligible_clients.is_empty() {
            return Ok(());
        }

        let notification = self.create_notification(&params)?;

        for client_id in eligible_clients {
            if let Err(_e) = self.transport.send(notification.clone().into(), client_id.clone()).await {}
        }

        Ok(())
    }

    fn get_eligible_clients(&self, level: &LoggingLevel) -> Vec<ConnectionId> {
        self.clients
            .iter()
            .filter_map(
                |entry| {
                    if should_receive_log_level(entry.value(), level) {
                        Some(entry.key().clone())
                    } else {
                        None
                    }
                },
            )
            .collect()
    }

    fn create_notification(
        &self,
        params: &LoggingMessageNotificationParams,
    ) -> Result<Notification, serde_json::Error> {
        let value = serde_json::to_value(params)?;

        match value {
            Value::Object(map) => Ok(Notification {
                jsonrpc: Some(jsonrpc_core::Version::V2),
                method: "notifications/message".to_string(),
                params: Params::Map(map),
            }),
            _ => Ok(Notification {
                jsonrpc: Some(jsonrpc_core::Version::V2),
                method: "notifications/message".to_string(),
                params: Params::Map(serde_json::Map::new()),
            }),
        }
    }
}

#[derive(Default)]
struct LogVisitor {
    message: Option<String>,
}

impl LogVisitor {
    fn new() -> Self {
        Self::default()
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

fn should_receive_log_level(client_level: &LoggingLevel, message_level: &LoggingLevel) -> bool {
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

impl<S> Layer<S> for McpLoggingLayer
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let target = event.metadata().target();

        if !target.starts_with(&self.target_prefix) {
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

        let layer = self.clone();
        let params = params.clone();

        tokio::spawn(async move { if let Err(_e) = layer.send_log(params).await {} });
    }
}
