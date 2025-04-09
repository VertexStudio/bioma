use anyhow::Error;
use bon::Builder;
use jsonrpc_core::{Notification, Params, Version};

use crate::schema::{ProgressNotificationParams, ProgressToken};
use crate::transport::TransportSender;
use crate::ConnectionId;

#[derive(Builder)]
pub struct Progress {
    sender: TransportSender,
    conn_id: ConnectionId,
    token: ProgressToken,
    progress: f64,
    total: Option<f64>,
    message: Option<String>,
}

impl Progress {
    pub fn new(sender: TransportSender, conn_id: ConnectionId, token: ProgressToken) -> Self {
        Self { sender, conn_id, token, progress: 0.0, total: None, message: None }
    }

    pub async fn increase(&mut self, increase: f64, message: Option<String>) -> Result<(), Error> {
        if increase <= 0.0 {
            return Err(anyhow::anyhow!("Progress increment must be positive"));
        }

        self.progress += increase;
        self.message = message;

        self.send().await
    }

    pub async fn update_to(&mut self, progress: f64, message: Option<String>) -> Result<(), Error> {
        if progress <= self.progress {
            return Err(anyhow::anyhow!("New progress value must be greater than current value"));
        }

        self.progress = progress;
        self.message = message;

        self.send().await
    }

    pub async fn send(&mut self) -> Result<(), Error> {
        let params = ProgressNotificationParams {
            progress_token: self.token.clone(),
            progress: self.progress,
            total: self.total,
            message: self.message.clone(),
        };

        let params_json = serde_json::to_value(params)?;

        let notification = Notification {
            jsonrpc: Some(Version::V2),
            method: "notifications/progress".to_string(),
            params: Params::Map(params_json.as_object().cloned().unwrap_or_default()),
        };

        self.sender
            .send(notification.into(), self.conn_id.clone())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send progress notification: {}", e))?;

        Ok(())
    }
}
