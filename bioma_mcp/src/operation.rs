use crate::schema::CancelledNotificationParams;
use crate::transport::TransportSender;
use crate::{MessageId, RequestId};
use anyhow::Error;
use serde::de::DeserializeOwned;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

pub struct Operation<T> {
    request_id: RequestId,
    future: Pin<Box<dyn Future<Output = Result<T, Error>> + Send>>,
    transport_sender: TransportSender,
}

impl<T> Operation<T> {
    pub fn new<F>(request_id: RequestId, future: F, transport_sender: TransportSender) -> Self
    where
        F: Future<Output = Result<T, Error>> + Send + 'static,
    {
        Self { request_id, future: Box::pin(future), transport_sender }
    }

    pub fn request_id(&self) -> &RequestId {
        &self.request_id
    }

    pub async fn cancel(&self, reason: Option<String>) -> Result<(), Error> {
        let (conn_id, message_id) = &self.request_id;
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

impl<T: DeserializeOwned> Future for Operation<T> {
    type Output = Result<T, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.future.as_mut().poll(cx)
    }
}
