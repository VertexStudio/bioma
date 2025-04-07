use crate::schema::CancelledNotificationParams;
use crate::transport::TransportSender;
use crate::{MessageId, RequestId};
use anyhow::Error;
use serde::de::DeserializeOwned;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

#[derive(Clone, Debug)]
pub enum OperationType {
    Single(RequestId),
    Multiple,
}

pub struct Operation<T> {
    operation_type: OperationType,
    future: Pin<Box<dyn Future<Output = Result<T, Error>> + Send>>,
    transport_sender: TransportSender,
    cancelled: AtomicBool,
}

impl<T> Operation<T> {
    pub fn new<F>(request_id: RequestId, future: F, transport_sender: TransportSender) -> Self
    where
        F: Future<Output = Result<T, Error>> + Send + 'static,
    {
        Self {
            operation_type: OperationType::Single(request_id),
            future: Box::pin(future),
            transport_sender,
            cancelled: AtomicBool::new(false),
        }
    }

    pub fn new_multiple<F>(future: F, transport_sender: TransportSender) -> Self
    where
        F: Future<Output = Result<T, Error>> + Send + 'static,
    {
        Self {
            operation_type: OperationType::Multiple,
            future: Box::pin(future),
            transport_sender,
            cancelled: AtomicBool::new(false),
        }
    }

    pub fn request_id(&self) -> Option<&RequestId> {
        match &self.operation_type {
            OperationType::Single(request_id) => Some(request_id),
            OperationType::Multiple => None,
        }
    }

    pub async fn cancel(&self, reason: Option<String>) -> Result<(), Error> {
        self.cancelled.store(true, Ordering::SeqCst);

        match &self.operation_type {
            OperationType::Multiple => Ok(()),
            OperationType::Single(request_id) => {
                let (conn_id, message_id) = request_id;
                let id_value = match message_id {
                    MessageId::Num(n) => serde_json::Value::Number(serde_json::Number::from(*n)),
                    MessageId::Str(s) => serde_json::Value::String(s.clone()),
                };

                let params = CancelledNotificationParams { request_id: id_value, reason };
                let params_json = serde_json::to_value(params)?;

                let notification = jsonrpc_core::Notification {
                    jsonrpc: Some(jsonrpc_core::Version::V2),
                    method: "notifications/cancelled".to_string(),
                    params: jsonrpc_core::Params::Map(params_json.as_object().cloned().unwrap_or_default()),
                };

                self.transport_sender
                    .send(notification.into(), conn_id.clone())
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to send cancel notification: {}", e))?;

                Ok(())
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

impl<T: DeserializeOwned> Future for Operation<T> {
    type Output = Result<T, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.is_cancelled() {
            return Poll::Ready(Err(anyhow::anyhow!("Operation cancelled")));
        }

        self.future.as_mut().poll(cx)
    }
}

impl<T> Drop for Operation<T> {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);

        if let OperationType::Single(request_id) = &self.operation_type {
            let (conn_id, message_id) = request_id.clone();
            let transport_sender = self.transport_sender.clone();

            let id_value = match &message_id {
                MessageId::Num(n) => serde_json::Value::Number(serde_json::Number::from(*n)),
                MessageId::Str(s) => serde_json::Value::String(s.clone()),
            };

            let params =
                CancelledNotificationParams { request_id: id_value, reason: Some("Operation dropped".to_string()) };

            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                match serde_json::to_value(params) {
                    Ok(params_json) => {
                        let notification = jsonrpc_core::Notification {
                            jsonrpc: Some(jsonrpc_core::Version::V2),
                            method: "notifications/cancelled".to_string(),
                            params: jsonrpc_core::Params::Map(params_json.as_object().cloned().unwrap_or_default()),
                        };

                        handle.spawn(async move {
                            if let Err(e) = transport_sender.send(notification.into(), conn_id).await {
                                tracing::error!("Failed to send cancellation notification on drop: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        tracing::error!("Failed to serialize cancellation params on drop: {}", e);
                    }
                }
            } else {
                tracing::warn!("Could not send cancellation notification on drop: no tokio runtime available");
            }
        }
    }
}
