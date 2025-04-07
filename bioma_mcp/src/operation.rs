use serde::de::DeserializeOwned;

use crate::RequestId;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

pub trait OperationRequest<E> {
    fn result<T: DeserializeOwned>(&self, request_id: &RequestId) -> impl Future<Output = Result<T, E>> + Send;
    fn cancel(&self, request_id: &RequestId, reason: Option<String>) -> impl Future<Output = Result<(), E>> + Send;
}

pub struct Operation<T, E, C: OperationRequest<E>> {
    request_id: RequestId,
    client: C,
    _marker_t: PhantomData<T>,
    _marker_e: PhantomData<E>,
}

impl<T: DeserializeOwned, E, C: OperationRequest<E>> Operation<T, E, C> {
    pub fn new(request_id: RequestId, client: C) -> Self {
        Self { request_id, client, _marker_t: PhantomData, _marker_e: PhantomData }
    }

    pub async fn result(&self) -> Result<T, E> {
        self.client.result::<T>(&self.request_id).await
    }

    pub async fn cancel(&self, reason: Option<String>) -> Result<(), E> {
        self.client.cancel(&self.request_id, reason).await
    }
}

impl<T: DeserializeOwned, E, C: OperationRequest<E>> Future for Operation<T, E, C> {
    type Output = Result<T, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        todo!()
    }
}
