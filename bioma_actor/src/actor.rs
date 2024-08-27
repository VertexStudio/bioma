use crate::engine::EE;
use crate::error::ActorError;
use derive_more::Display;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::any::type_name;
use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use surrealdb::{
    sql::{Id, Thing},
    Action, Notification,
};
use tracing::{debug, error};

const DB_TABLE_ACTOR: &str = "actor";
const DB_TABLE_MESSAGE: &str = "message";
const DB_TABLE_REPLY: &str = "reply";

/// The message frame that is sent between actors
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    id: Thing,
    pub name: Cow<'static, str>,
    pub tx: Thing,
    pub rx: Thing,
    pub msg: Value,
}

pub trait Message<T>: ActorModel
where
    T: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync,
{
    type Response: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync;

    fn handle(&mut self, message: &T) -> impl Future<Output = Result<Self::Response, ActorError>>;
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, Hash, PartialEq, Display)]
pub struct Record {
    id: Thing,
}

/// A unique identifier for a distributed actor
#[derive(Clone, Debug, Serialize, Deserialize, Eq, Hash, PartialEq)]

pub struct ActorId {
    id: Thing,
    #[serde(default)]
    kind: Option<Cow<'static, str>>,
}

impl std::fmt::Display for ActorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            Some(kind) => write!(f, "{}:{}", self.id, kind),
            None => write!(f, "{}", self.id),
        }
    }
}

impl ActorId {
    /// Create an actor id to reference some actor, does not create the actor in the database
    pub fn new(uid: impl Into<Cow<'static, str>>) -> Self {
        let id = Thing::from((DB_TABLE_ACTOR, Id::String(uid.into().to_string())));
        Self { id, kind: None }
    }

    /// Create an actor id to reference some actor of type T
    pub fn of<T: ActorModel>(uid: impl Into<Cow<'static, str>>) -> Self {
        let id = Thing::from((DB_TABLE_ACTOR, Id::String(uid.into().to_string())));
        Self { id, kind: Some(type_name::<T>().into()) }
    }

    /// Create a new distributed actor
    pub async fn spawn(uid: impl Into<Cow<'static, str>>) -> Result<Self, ActorError> {
        let id = Thing::from((DB_TABLE_ACTOR, Id::String(uid.into().to_string())));
        let _actor: Vec<Record> =
            EE.db().create(DB_TABLE_ACTOR).content(ActorId { id: id.clone(), kind: None }).await?;
        Ok(Self { id, kind: None })
    }

    /// Create a new distributed actor of type T
    pub async fn spawn_of<T: ActorModel>(uid: impl Into<Cow<'static, str>>) -> Result<Self, ActorError> {
        let id = Thing::from((DB_TABLE_ACTOR, Id::String(uid.into().to_string())));
        let kind: Cow<'static, str> = type_name::<T>().into();
        let _actor: Vec<Record> =
            EE.db().create(DB_TABLE_ACTOR).content(ActorId { id: id.clone(), kind: Some(kind.clone()) }).await?;
        Ok(Self { id, kind: Some(kind) })
    }

    /// Check if the actor is healthy, exists and is reachable
    pub async fn health(&self) -> bool {
        let actor: Result<Option<ActorId>, _> = EE.db().select(&self.id).await;
        match actor {
            Ok(Some(_)) => true,
            _ => false,
        }
    }
}

/// A distributed actor
pub trait ActorModel {
    fn id(&self) -> &ActorId;
    fn start(&mut self) -> impl Future<Output = Result<(), ActorError>>;

    fn send<M, T>(&self, message: T, to: &ActorId) -> impl Future<Output = Result<M::Response, ActorError>>
    where
        M: Message<T>,
        T: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync,
    {
        async move {
            let value = serde_json::to_value(&message)?;
            let response = self.id().send(type_name::<T>(), to, &value).await?;
            let response = serde_json::from_value(response)?;
            Ok(response)
        }
    }

    fn reply<M, T>(&self, request: &Frame, message: M::Response) -> impl Future<Output = Result<(), ActorError>>
    where
        M: Message<T>,
        T: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync,
    {
        async move {
            let value = serde_json::to_value(&message)?;
            self.id().reply(request, value).await?;
            Ok(())
        }
    }

    fn recv(&self) -> impl Future<Output = Result<MessageStream, ActorError>> {
        self.id().recv()
    }

    fn is<M, T>(&self, frame: &Frame) -> Option<T>
    where
        M: Message<T>,
        T: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync,
    {
        if frame.name == type_name::<T>() {
            serde_json::from_value(frame.msg.clone()).ok()
        } else {
            None
        }
    }
}

pub type MessageStream = Pin<Box<dyn Stream<Item = Result<Frame, surrealdb::Error>> + Send>>;

impl Protocol for ActorId {
    fn id(&self) -> &Thing {
        &self.id
    }
}

pub trait Protocol {
    /// Get the actor id
    fn id(&self) -> &Thing;

    /// Send a message to the actor and wait for a reply
    fn send(
        &self,
        name: impl Into<Cow<'static, str>>,
        to: &ActorId,
        message: &Value,
    ) -> impl Future<Output = Result<Value, ActorError>> {
        async move {
            let name = name.into();
            let msg_id = Id::ulid();
            let msg_str = serde_json::to_string(&message)?;
            let request_id = Thing::from((DB_TABLE_MESSAGE, msg_id.clone()));
            let reply_id = Thing::from((DB_TABLE_REPLY, msg_id.clone()));

            let request = Frame {
                id: request_id.clone(),
                name: name.clone(),
                tx: self.id().clone(),
                rx: to.id.clone(),
                msg: message.clone(),
            };

            debug!("[{}] msg-send {} {} {} {}", self.id(), name, request.id, to.id, msg_str);

            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(0)).await;
                let msg_ids: Result<Vec<Record>, _> = EE.db().create(DB_TABLE_MESSAGE).content(request).await;
                if let Ok(msg_ids) = msg_ids {
                    let id = msg_ids[0].id.clone();
                    if request_id != id {
                        error!("msg-send {}", request_id);
                    }
                }
            });

            // Live wait for reply
            let mut stream = EE.db().select(reply_id).live().await?;
            let Some(notification) = stream.next().await else {
                return Err(ActorError::LiveStream(format!("Empty: {}", name).into()));
            };
            let notification: Notification<Frame> = notification?;
            match notification.action {
                Action::Create => {
                    let data = &notification.data;
                    let msg_value = serde_json::to_string(&data.msg)?;
                    debug!("[{}] msg-done {} {} {} {}", self.id(), data.name, data.id, data.rx, msg_value);
                    Ok(data.msg.clone())
                }
                _ => Err(ActorError::LiveStream(format!("!Action::Create: {}", name).into())),
            }
        }
    }

    /// Reply to a message
    fn reply(&self, request: &Frame, message: Value) -> impl Future<Output = Result<(), ActorError>> {
        async move {
            // Use the request id as the reply id
            let reply_id = Thing::from((DB_TABLE_REPLY, request.id.id.clone()));
            let reply_msg = serde_json::to_string(&message)?;

            // Assert request.rx == self, can only reply to messages sent to us
            if request.rx != *self.id() {
                return Err(ActorError::IdMismatch(request.tx.clone(), self.id().clone()));
            }

            let reply = Frame {
                id: reply_id.clone(),
                name: request.name.clone(),
                tx: request.rx.clone(),
                rx: request.tx.clone(),
                msg: message,
            };

            debug!("[{}] msg-rply {} {} {} {}", self.id(), reply.name, reply_id, reply.tx, reply_msg);

            let _entry: Vec<Record> = EE.db().create(DB_TABLE_REPLY).content(reply).await?;

            Ok(())
        }
    }

    /// Receive messages
    fn recv(&self) -> impl Future<Output = Result<MessageStream, ActorError>> {
        async move {
            let query = format!("LIVE SELECT * FROM {} WHERE rx = {}", DB_TABLE_MESSAGE, self.id());
            debug!("[{}] msg-live {}", self.id(), &query);
            let mut res = EE.db().query(&query).await?;
            let live_query = res.stream::<Notification<Frame>>(0)?;
            let self_id = self.id().clone();
            let live_query = live_query
                .filter(|item| {
                    // Filter out non-create actions
                    let should_filter =
                        item.as_ref().ok().map(|notification| notification.action == Action::Create).unwrap_or(false);
                    async move { should_filter }
                })
                // Map the notification to a frame
                .map(|item| {
                    let item = item?;
                    Ok(item.data)
                })
                .inspect(move |item| match item {
                    Ok(frame) => {
                        debug!(
                            "[{}] msg-recv {} {} {} -> {} {}",
                            self_id,
                            frame.name,
                            frame.id,
                            frame.tx,
                            frame.rx,
                            serde_json::to_string(&frame.msg).unwrap_or("".into())
                        );
                    }
                    Err(error) => debug!("msg-recv {} {:?}", self_id, error),
                });
            Ok(Box::pin(live_query) as MessageStream)
        }
    }
}
