use crate::{ConnectionId, RequestId};
use std::future::Future;
use std::pin::Pin;
use tokio::sync::oneshot;

/// Operation represents an in-flight request from either client or server.
/// It provides a unified way to track, await, and cancel requests.
pub struct Operation<R, E> {
    future: Pin<Box<dyn Future<Output = Result<R, E>> + Send>>,
    cancel_tx: oneshot::Sender<Option<String>>,
    conn_id: ConnectionId,
    request_id: RequestId,
}

impl<R, E> Operation<R, E> {
    /// Creates a new operation with the given future, cancellation channel, connection ID, and request ID
    pub fn new(
        future: Pin<Box<dyn Future<Output = Result<R, E>> + Send>>,
        cancel_tx: oneshot::Sender<Option<String>>,
        conn_id: ConnectionId,
        request_id: RequestId,
    ) -> Self {
        Self { future, cancel_tx, conn_id, request_id }
    }

    /// Returns the connection ID associated with this operation
    pub fn connection_id(&self) -> &ConnectionId {
        &self.conn_id
    }

    /// Returns the request ID associated with this operation
    pub fn request_id(&self) -> &RequestId {
        &self.request_id
    }

    /// Cancels the operation with an optional reason
    pub async fn cancel(self, reason: Option<String>) -> Result<(), E>
    where
        E: From<&'static str> + Send + 'static,
    {
        self.cancel_tx.send(reason).map_err(|_| From::from("Failed to send cancellation signal"))
    }
}

// Implement Future for Operation to allow .await syntax
impl<R, E> Future for Operation<R, E>
where
    E: Send + 'static,
    R: Send + 'static,
{
    type Output = Result<R, E>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        self.future.as_mut().poll(cx)
    }
}

// Type aliases for convenience
pub type ClientOperation<R> = Operation<R, crate::client::ClientError>;
pub type ServerOperation<R> = Operation<R, crate::server::ServerError>;
