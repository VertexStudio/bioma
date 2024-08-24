use crate::engine::DB;
use crate::error::ActorError;
use derive_more::Display;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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

/// The raw message type that is sent between actors
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    id: Thing,
    pub name: Cow<'static, str>,
    pub tx: Thing,
    pub rx: Thing,
    pub data: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, Hash, PartialEq, Display)]
pub struct Record {
    id: Thing,
}

/// A unique identifier for a distributed actor
#[derive(Clone, Debug, Serialize, Deserialize, Eq, Hash, PartialEq, Display)]
pub struct ActorId {
    id: Thing,
}

/// A distributed actor
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Actor {
    id: Thing,
}

impl ActorId {
    /// Create a new distributed actor
    pub async fn new(uid: impl Into<Cow<'static, str>>) -> Result<Self, ActorError> {
        let id = Thing::from((DB_TABLE_ACTOR, Id::String(uid.into().to_string())));
        let actor = Actor { id: id.clone() };
        let _actor: Vec<Record> = DB.create(DB_TABLE_ACTOR).content(actor).await?;
        Ok(Self { id })
    }

    /// Check if the actor is healthy, exists and is reachable
    pub async fn health(&self) -> bool {
        let actor: Result<Option<Actor>, _> = DB.select(&self.id).await;
        match actor {
            Ok(Some(_)) => true,
            _ => false,
        }
    }
}

pub type MessageStream = Pin<Box<dyn Stream<Item = Result<Notification<Message>, surrealdb::Error>> + Send>>;

impl ActorProtocol for ActorId {
    fn id(&self) -> &Thing {
        &self.id
    }
}

pub trait ActorProtocol {
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

            let request = Message {
                id: request_id.clone(),
                name: name.clone(),
                tx: self.id().clone(),
                rx: to.id.clone(),
                data: message.clone(),
            };

            debug!("[{}] msg-send {} {} {} {}", self.id(), name, request.id, to, msg_str);

            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(0)).await;
                let msg_ids: Result<Vec<Record>, _> = DB.create(DB_TABLE_MESSAGE).content(request).await;
                if let Ok(msg_ids) = msg_ids {
                    let id = msg_ids[0].id.clone();
                    if request_id != id {
                        error!("msg-send {}", request_id);
                    }
                }
            });

            // Live wait for reply
            let mut stream = DB.select(reply_id).live().await?;
            let Some(notification) = stream.next().await else {
                return Err(ActorError::LiveStream(format!("Empty: {}", name).into()));
            };
            let notification: Notification<Message> = notification?;
            match notification.action {
                Action::Create => {
                    let data = &notification.data;
                    let msg_value = serde_json::to_string(&data.data)?;
                    debug!("[{}] msg-done {} {} {} {}", self.id(), data.name, data.id, data.rx, msg_value);
                    Ok(data.data.clone())
                }
                _ => Err(ActorError::LiveStream(format!("!Action::Create: {}", name).into())),
            }
        }
    }

    /// Reply to a message
    fn reply(&self, request: &Message, message: Value) -> impl Future<Output = Result<(), ActorError>> {
        async move {
            // Use the request id as the reply id
            let reply_id = Thing::from((DB_TABLE_REPLY, request.id.id.clone()));
            let reply_msg = serde_json::to_string(&message)?;

            // Assert request.rx == self, can only reply to messages sent to us
            if request.rx != *self.id() {
                return Err(ActorError::IdMismatch(request.tx.clone(), self.id().clone()));
            }

            let reply = Message {
                id: reply_id.clone(),
                name: request.name.clone(),
                tx: request.rx.clone(),
                rx: request.tx.clone(),
                data: message,
            };

            debug!("[{}] msg-rply {} {} {} {}", self.id(), reply.name, reply_id, reply.tx, reply_msg);

            let _entry: Vec<Record> = DB.create(DB_TABLE_REPLY).content(reply).await?;

            Ok(())
        }
    }

    /// Receive messages
    fn recv(&self) -> impl Future<Output = Result<MessageStream, ActorError>> {
        async move {
            let query = format!("LIVE SELECT * FROM {} WHERE rx = {}", DB_TABLE_MESSAGE, self.id());
            debug!("[{}] msg-live {}", self.id(), &query);
            let mut res = DB.query(&query).await?;
            let live_query = res.stream::<Notification<Message>>(0)?;
            let self_id = self.id().clone();
            let live_query = live_query
                .filter(|item| {
                    // Filter out non-create actions
                    let should_filter =
                        item.as_ref().ok().map(|notification| notification.action == Action::Create).unwrap_or(false);
                    async move { should_filter }
                })
                .inspect(move |item| match item {
                    Ok(notification) => {
                        let message = &notification.data;
                        debug!(
                            "[{}] msg-recv {} {} {} -> {}{}",
                            self_id,
                            message.name,
                            message.id,
                            message.tx,
                            message.rx,
                            serde_json::to_string(&message.data).unwrap_or("".into())
                        );
                    }
                    Err(error) => debug!("msg-recv {} {:?}", self_id, error),
                });
            Ok(Box::pin(live_query) as MessageStream)
        }
    }
}
