use crate::schema::{CancelledNotificationParams, ProgressNotificationParams};
use crate::transport::TransportSender;
use crate::{MessageId, RequestId};
use anyhow::Error;
use futures::stream::Stream;
use serde::de::DeserializeOwned;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub enum OperationType {
    Single {
        request_id: RequestId,
        transport_sender: TransportSender,
        progress_rx: Option<mpsc::Receiver<ProgressNotificationParams>>,
    },
    Sub {
        request_id: RequestId,
        transport_sender: TransportSender,
        completed: bool,
    },
    Multiple {
        progress_rx: Option<mpsc::Receiver<ProgressNotificationParams>>,
    },
}

pub struct Operation<T> {
    operation_type: OperationType,
    future: Pin<Box<dyn Future<Output = Result<T, Error>> + Send>>,
    cancel_token: CancellationToken,
}

impl<T> Operation<T> {
    pub fn new<F>(
        request_id: RequestId,
        future: F,
        transport_sender: TransportSender,
        progress_rx: Option<mpsc::Receiver<ProgressNotificationParams>>,
    ) -> Self
    where
        F: Future<Output = Result<T, Error>> + Send + 'static,
    {
        let cancel_token = CancellationToken::new();
        let token_clone = cancel_token.clone();

        let wrapped_future = async move {
            tokio::select! {
                result = future => result,
                _ = token_clone.cancelled() => {
                    Err(anyhow::anyhow!("Operation cancelled"))
                }
            }
        };

        Self {
            operation_type: OperationType::Single { request_id, transport_sender, progress_rx },
            future: Box::pin(wrapped_future),
            cancel_token,
        }
    }

    pub fn new_sub<F>(request_id: RequestId, future: F, transport_sender: TransportSender) -> Self
    where
        F: Future<Output = Result<T, Error>> + Send + 'static,
    {
        let cancel_token = CancellationToken::new();
        let token_clone = cancel_token.clone();

        let wrapped_future = async move {
            tokio::select! {
                result = future => result,
                _ = token_clone.cancelled() => {
                    Err(anyhow::anyhow!("Operation cancelled"))
                }
            }
        };

        Self {
            operation_type: OperationType::Sub { request_id, transport_sender, completed: false },
            future: Box::pin(wrapped_future),
            cancel_token,
        }
    }

    pub fn new_multiple<F>(future: F, progress_rx: Option<mpsc::Receiver<ProgressNotificationParams>>) -> Self
    where
        F: Future<Output = Result<T, Error>> + Send + 'static,
    {
        let cancel_token = CancellationToken::new();
        let token_clone = cancel_token.clone();

        let wrapped_future = async move {
            tokio::select! {
                result = future => result,
                _ = token_clone.cancelled() => {
                    Err(anyhow::anyhow!("Operation cancelled"))
                }
            }
        };

        Self { operation_type: OperationType::Multiple { progress_rx }, future: Box::pin(wrapped_future), cancel_token }
    }

    pub async fn cancel(&self, reason: Option<String>) -> Result<(), Error> {
        self.cancel_token.cancel();

        match &self.operation_type {
            OperationType::Multiple { .. } => Ok(()),
            OperationType::Single { request_id, transport_sender, .. }
            | OperationType::Sub { request_id, transport_sender, .. } => {
                let (conn_id, message_id) = request_id;
                let id_value = match message_id {
                    MessageId::Num(n) => serde_json::Value::Number(serde_json::Number::from(*n as u64)),
                    MessageId::Str(s) => serde_json::Value::String(s.clone()),
                };

                let params = CancelledNotificationParams { request_id: id_value, reason };
                let params_json = serde_json::to_value(params)?;

                let notification = jsonrpc_core::Notification {
                    jsonrpc: Some(jsonrpc_core::Version::V2),
                    method: "notifications/cancelled".to_string(),
                    params: jsonrpc_core::Params::Map(params_json.as_object().cloned().unwrap_or_default()),
                };

                transport_sender
                    .send(notification.into(), conn_id.clone())
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to send cancel notification: {}", e))?;

                Ok(())
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }

    pub fn recv(&mut self) -> Pin<Box<dyn Stream<Item = ProgressNotificationParams> + Send>> {
        let progress_rx = match &mut self.operation_type {
            OperationType::Single { progress_rx, .. } => progress_rx.take(),
            OperationType::Multiple { progress_rx } => progress_rx.take(),
            OperationType::Sub { .. } => None,
        };

        Box::pin(futures::stream::unfold(progress_rx, |rx| async move {
            match rx {
                Some(mut receiver) => match receiver.recv().await {
                    Some(notification) => Some((notification, Some(receiver))),
                    None => None,
                },
                None => None,
            }
        }))
    }
}

impl<T: DeserializeOwned> Future for Operation<T> {
    type Output = Result<T, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let result = self.future.as_mut().poll(cx);

        if let Poll::Ready(_) = result {
            if let OperationType::Sub { completed, .. } = &mut self.operation_type {
                *completed = true;
            }
        }

        result
    }
}

impl<T> Drop for Operation<T> {
    fn drop(&mut self) {
        if let OperationType::Sub { request_id, transport_sender, completed } = &self.operation_type {
            if *completed {
                return;
            }

            self.cancel_token.cancel();

            let (conn_id, message_id) = request_id.clone();
            let transport_sender = transport_sender.clone();

            let id_value = match message_id {
                MessageId::Num(n) => serde_json::Value::Number(serde_json::Number::from(n as u64)),
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
