use crate::engine::Engine;
use crate::error::ActorError;
use derive_more::Display;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::any::type_name;
use std::borrow::Cow;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use surrealdb::{
    sql::{Id, Thing},
    Action, Notification,
};
use tracing::{debug, error};

// Constants for database table names
const DB_TABLE_ACTOR: &str = "actor";
const DB_TABLE_MESSAGE: &str = "message";
const DB_TABLE_REPLY: &str = "reply";

/// The message frame that is sent between actors
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    /// Message id
    id: Thing,
    /// Message name (usually a type name)
    pub name: Cow<'static, str>,
    /// Sender
    pub tx: Thing,
    /// Receiver
    pub rx: Thing,
    /// Message content
    pub msg: Value,
}

pub type MessageStream = Pin<Box<dyn Stream<Item = Result<Frame, ActorError>> + Send>>;

/// Message handling behavior
/// Actors implement this trait to handle messages
pub trait Message<T>: Actor
where
    T: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync,
{
    type Response: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync;

    fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: &T,
    ) -> impl Future<Output = Result<Self::Response, ActorError>>;

    fn reply(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: &T,
        frame: &Frame,
    ) -> impl Future<Output = Result<Self::Response, ActorError>> {
        async move {
            let response = self.handle(ctx, message).await?;
            ctx.reply::<Self, T>(frame, response.clone()).await?;
            Ok(response)
        }
    }
}

// Record struct for database entries
#[derive(Clone, Debug, Serialize, Deserialize, Eq, Hash, PartialEq, Display)]
struct Record {
    id: Thing,
}

/// A unique identifier for a distributed actor
#[derive(Clone, Debug, Serialize, Deserialize, Eq, Hash, PartialEq)]
pub struct ActorId {
    id: Thing,
    kind: Cow<'static, str>,
}

impl std::fmt::Display for ActorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", &self.id.id)
    }
}

impl ActorId {
    /// Create an actor id to reference some actor
    pub fn of<T: Actor>(uid: impl Into<Cow<'static, str>>) -> Self {
        let id = Thing::from((DB_TABLE_ACTOR, Id::String(uid.into().to_string())));
        Self { id, kind: type_name::<T>().into() }
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }
}

/// Implement this trait to define an actor
pub trait Actor: Sized + Clone + Serialize + for<'de> Deserialize<'de> + 'static + Debug {
    // Spawn a new actor
    fn spawn(
        engine: &Engine,
        id: &ActorId,
        actor: Self,
    ) -> impl Future<Output = Result<ActorContext<Self>, ActorError>> {
        async move {
            // Check if the actor kind matches
            if id.kind != type_name::<Self>() {
                return Err(ActorError::ActorKindMismatch(id.kind.clone(), type_name::<Self>().into()));
            }

            // Create actor record in the database
            let content = ActorRecord { id: id.id.clone(), kind: type_name::<Self>().into(), state: actor.clone() };
            let _record: Vec<Record> = engine.db().create(DB_TABLE_ACTOR).content(content).await?;

            // Create and return the actor context
            let ctx = ActorContext::new(engine.clone(), id.clone(), actor);
            Ok(ctx)
        }
    }

    // Start the actor
    fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), ActorError>>;
}

/// Database record for an actor
#[derive(Clone, Debug, Serialize)]
pub struct ActorRecord<T: Actor> {
    id: Thing,
    kind: Cow<'static, str>,
    state: T,
}

/// Context for an actor, that binds an actor to its engine
#[derive(Clone, Debug)]
pub struct ActorContext<T: Actor> {
    engine: Engine,
    id: ActorId,
    actor: T,
}

impl<T: Actor> ActorContext<T> {
    fn new(engine: Engine, id: ActorId, actor: T) -> Self {
        Self { engine, id, actor }
    }

    /// Start the actor
    pub async fn start(&mut self) -> Result<(), ActorError> {
        let mut actor = self.actor.clone();
        actor.start(self).await?;
        self.actor = actor;
        Ok(())
    }
}

impl<T: Actor> ActorModel for ActorContext<T> {
    fn id(&self) -> &ActorId {
        &self.id
    }

    fn engine(&self) -> &Engine {
        &self.engine
    }
}

/// Core functionality of a distributed actor
pub trait ActorModel {
    /// Get the actor id
    fn id(&self) -> &ActorId;

    /// Get the engine
    fn engine(&self) -> &Engine;

    /// Check the health of the actor
    fn health(&self) -> impl Future<Output = bool> {
        async move { self.engine().health().await }
    }

    /// Send a message to another actor and return without waiting for a reply
    fn do_send<M, T>(&self, message: T, to: &ActorId) -> impl Future<Output = Result<(), ActorError>>
    where
        M: Message<T>,
        T: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync,
    {
        async move {
            // Serialize the message
            let msg_value = serde_json::to_value(&message)?;

            let name = type_name::<T>();
            let msg_id = Id::ulid();
            let request_id = Thing::from((DB_TABLE_MESSAGE, msg_id.clone()));

            // Create the frame for the message
            let request = Frame {
                id: request_id.clone(),
                name: name.into(),
                tx: self.id().id.clone(),
                rx: to.id.clone(),
                msg: msg_value.clone(),
            };

            debug!("[{}] msg-send {} {} {} {}", &self.id().id, name, &request.id, &to.id, &msg_value);

            let msg_engine = self.engine().clone();

            let msg_ids: Result<Vec<Record>, _> = msg_engine.db().create(DB_TABLE_MESSAGE).content(request).await;
            if let Ok(msg_ids) = msg_ids {
                let id = msg_ids[0].id.clone();
                if request_id != id {
                    error!("msg-send {}", request_id);
                }
            }

            Ok(())
        }
    }

    /// Send a message to another actor and wait for a reply
    fn send<M, T>(&self, message: T, to: &ActorId) -> impl Future<Output = Result<M::Response, ActorError>>
    where
        M: Message<T>,
        T: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync,
    {
        async move {
            // Serialize the message
            let msg_value = serde_json::to_value(&message)?;

            let name = type_name::<T>();
            let msg_id = Id::ulid();
            let request_id = Thing::from((DB_TABLE_MESSAGE, msg_id.clone()));
            let reply_id = Thing::from((DB_TABLE_REPLY, msg_id.clone()));

            // Create the frame for the message
            let request = Frame {
                id: request_id.clone(),
                name: name.into(),
                tx: self.id().id.clone(),
                rx: to.id.clone(),
                msg: msg_value.clone(),
            };

            debug!("[{}] msg-send {} {} {} {}", &self.id().id, name, &request.id, &to.id, &msg_value);

            let msg_engine = self.engine().clone();

            // Spawn a task to create the message in the database
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(0)).await;
                let msg_ids: Result<Vec<Record>, _> = msg_engine.db().create(DB_TABLE_MESSAGE).content(request).await;
                if let Ok(msg_ids) = msg_ids {
                    let id = msg_ids[0].id.clone();
                    if request_id != id {
                        error!("msg-send {}", request_id);
                    }
                }
            });

            // Wait for the reply
            let mut stream = self.engine().db().select(reply_id).live().await?;
            let Some(notification) = stream.next().await else {
                return Err(ActorError::LiveStream(format!("Empty: {}", name).into()));
            };
            let notification: Notification<Frame> = notification?;
            let response = match notification.action {
                Action::Create => {
                    let data = &notification.data;
                    debug!("[{}] msg-done {} {} {} {}", &self.id().id, &data.name, &data.id, &data.rx, &data.msg);
                    Ok(data.msg.clone())
                }
                _ => Err(ActorError::LiveStream(format!("!Action::Create: {}", name).into())),
            }?;

            let response = serde_json::from_value(response)?;
            Ok(response)
        }
    }

    /// Reply to a received message
    fn reply<M, T>(&self, request: &Frame, message: M::Response) -> impl Future<Output = Result<(), ActorError>>
    where
        M: Message<T>,
        T: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync,
    {
        async move {
            let msg_value = serde_json::to_value(&message)?;

            // Use the request id as the reply id
            let reply_id = Thing::from((DB_TABLE_REPLY, request.id.id.clone()));

            // Assert request.rx == self, can only reply to messages sent to us
            if request.rx != self.id().id {
                return Err(ActorError::IdMismatch(request.tx.clone(), self.id().id.clone()));
            }

            let reply = Frame {
                id: reply_id.clone(),
                name: request.name.clone(),
                tx: request.rx.clone(),
                rx: request.tx.clone(),
                msg: msg_value.clone(),
            };

            debug!("[{}] msg-rply {} {} {} {}", &self.id().id, &reply.name, &reply_id, &reply.tx, &msg_value);

            let _entry: Vec<Record> = self.engine().db().create(DB_TABLE_REPLY).content(reply).await?;
            Ok(())
        }
    }

    /// Receive messages for this actor
    fn recv(&self) -> impl Future<Output = Result<MessageStream, ActorError>> {
        async move {
            let query = format!("LIVE SELECT * FROM {} WHERE rx = {}", DB_TABLE_MESSAGE, self.id().id);
            debug!("[{}] msg-live {}", &self.id().id, &query);
            let mut res = self.engine().db().query(&query).await?;
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
                            &self_id.id, &frame.name, &frame.id, &frame.tx, &frame.rx, &frame.msg
                        );
                    }
                    Err(error) => debug!("msg-recv {} {:?}", self_id.id, error),
                });
            Ok(Box::pin(live_query) as MessageStream)
        }
    }

    /// Check if a received frame matches a specific message type
    /// and deserialize it into the message type.
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

#[cfg(test)]
mod tests {
    use crate::dbg_export_db;
    use crate::prelude::*;
    use futures::StreamExt;
    use serde::{Deserialize, Serialize};
    use std::any::type_name;
    use std::future::Future;
    use test_log::test;
    use tracing::info;

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct Ping;

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct Pong {
        times: usize,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct PingActor {
        pong_id: ActorId,
    }

    impl Actor for PingActor {
        fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), ActorError>> {
            async move {
                info!("{} says hi!", type_name::<Self>());
                let pong_id = self.pong_id.clone();
                loop {
                    info!("ping");
                    let pong = ctx.send::<PongActor, Ping>(Ping, &pong_id).await?;
                    if pong.times == 0 {
                        break;
                    }
                }
                info!("{} says bye!", type_name::<Self>());
                Ok(())
            }
        }
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct PongActor {
        times: usize,
    }

    impl Actor for PongActor {
        fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), ActorError>> {
            async move {
                info!("{} says hi!", type_name::<Self>());
                let mut stream = ctx.recv().await?;
                while let Some(Ok(frame)) = stream.next().await {
                    if let Some(message) = ctx.is::<Self, Ping>(&frame) {
                        self.times -= 1;
                        let _response = self.reply(ctx, &message, &frame).await?;
                        info!("pong");
                        if self.times == 0 {
                            break;
                        }
                    }
                }
                info!("{} says bye!", type_name::<Self>());
                Ok(())
            }
        }
    }

    impl Message<Ping> for PongActor {
        type Response = Pong;

        fn handle(
            &mut self,
            _ctx: &mut ActorContext<Self>,
            _message: &Ping,
        ) -> impl Future<Output = Result<Self::Response, ActorError>> {
            async move { Ok(Pong { times: self.times }) }
        }
    }

    #[test(tokio::test)]
    async fn test_actor_ping_pong() -> Result<(), ActorError> {
        let engine = Engine::test().await?;

        // Create actor_ids for the ping and pong actors
        let pong_id = ActorId::of::<PongActor>("/pong");
        let ping_id = ActorId::of::<PingActor>("/ping");

        // Spawn the ping and pong actors
        let mut ping_actor = Actor::spawn(&engine, &ping_id, PingActor { pong_id: pong_id.clone() }).await?;
        let mut pong_actor = Actor::spawn(&engine, &pong_id, PongActor { times: 3 }).await?;

        // Check health of the actors
        let ping_health = ping_actor.health().await;
        let pong_health = pong_actor.health().await;
        assert!(ping_health);
        assert!(pong_health);

        // Start the ping and pong actors
        let ping_handle = tokio::spawn(async move {
            ping_actor.start().await.unwrap();
        });
        let pong_handle = tokio::spawn(async move {
            pong_actor.start().await.unwrap();
        });

        // Wait for the ping and pong actors to finish
        let _ = ping_handle.await;
        let _ = pong_handle.await;

        // Export the database for debugging
        dbg_export_db!(engine);

        Ok(())
    }
}
