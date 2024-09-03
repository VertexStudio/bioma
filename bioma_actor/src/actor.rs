use crate::engine::Engine;
use derive_more::Display;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::any::type_name;
use std::borrow::Cow;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use surrealdb::{sql::Id, value::RecordId, Action, Notification};
use tracing::{debug, error};

// Constants for database table names
const DB_TABLE_ACTOR: &str = "actor";
const DB_TABLE_MESSAGE: &str = "message";
const DB_TABLE_REPLY: &str = "reply";

pub trait ActorError: std::error::Error + Debug + Send + Sync + 'static + From<SystemActorError> {}

/// Enumerates the types of errors that can occur in Actor framework
#[derive(thiserror::Error, Debug)]
pub enum SystemActorError {
    // IO error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Engine error: {0}")]
    EngineError(#[from] surrealdb::Error),
    #[error("MessageTypeMismatch: {0}")]
    MessageTypeMismatch(&'static str),
    #[error("LiveStream error: {0}")]
    LiveStream(Cow<'static, str>),
    // json_serde::Error
    #[error("JsonSerde error: {0}")]
    JsonSerde(#[from] serde_json::Error),
    // Id mismatch a and b
    #[error("Id mismatch: {0:?} {1:?}")]
    IdMismatch(RecordId, RecordId),
    #[error("Actor kind mismatch: {0} {1}")]
    ActorKindMismatch(Cow<'static, str>, Cow<'static, str>),
}

impl ActorError for SystemActorError {}

/// The message frame that is sent between actors
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    /// Message id
    id: RecordId,
    /// Message name (usually a type name)
    pub name: Cow<'static, str>,
    /// Sender
    pub tx: RecordId,
    /// Receiver
    pub rx: RecordId,
    /// Message content
    pub msg: Value,
}

impl Frame {
    /// Check if this frame matches a specific message type
    /// and deserialize it into the message type.
    pub fn is<M>(&self) -> Option<M>
    where
        M: Clone + Serialize + for<'de> Deserialize<'de> + Send + Sync,
    {
        if self.name == type_name::<M>() {
            serde_json::from_value(self.msg.clone()).ok()
        } else {
            None
        }
    }
}

pub type MessageStream = Pin<Box<dyn Stream<Item = Result<Frame, SystemActorError>> + Send>>;

/// Message handling behavior
/// Actors implement this trait to handle messages
pub trait Message<T>: Actor
where
    T: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync,
{
    type Response: Clone + Serialize + for<'de> Deserialize<'de> + Send + Sync;

    fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: &T,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>>;

    fn reply(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: &T,
        frame: &Frame,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> {
        async move {
            let response = self.handle(ctx, message).await?;
            ctx.reply::<Self, T>(frame, response.clone()).await?;
            Ok(response)
        }
    }
}

// Record struct for database entries
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Display)]
struct Record {
    id: RecordId,
}

/// A unique identifier for a distributed actor
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ActorId {
    id: RecordId,
    kind: Cow<'static, str>,
}

impl std::fmt::Display for ActorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", &self.id)
    }
}

// impl surrealdb::opt::IntoResource<Result<RecordId, surrealdb::Error>> for ActorId {
//     fn into_resource(self) -> Result<RecordId, surrealdb::Error> {
//         let record_id = surrealdb::opt::Resource::RecordId(self.id);
//         Ok(self.id)
//     }
// }

impl ActorId {
    /// Create an actor id to reference some actor
    pub fn of<T: Actor>(uid: impl Into<Cow<'static, str>>) -> Self {
        let id = RecordId::from_table_key(DB_TABLE_ACTOR, uid.into().to_string());
        Self { id, kind: type_name::<T>().into() }
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }
}

/// Implement this trait to define an actor
pub trait Actor: Sized + Clone + Serialize + for<'de> Deserialize<'de> + 'static + Debug {
    type Error: ActorError;

    // Spawn a new actor
    fn spawn(
        engine: &Engine,
        id: &ActorId,
        actor: Self,
    ) -> impl Future<Output = Result<ActorContext<Self>, Self::Error>> {
        async move {
            // Check if the actor kind matches
            if id.kind != type_name::<Self>() {
                return Err(Self::Error::from(SystemActorError::ActorKindMismatch(
                    id.kind.clone(),
                    type_name::<Self>().into(),
                )));
            }

            // Create actor record in the database
            let content = ActorRecord { id: id.id.clone(), kind: type_name::<Self>().into(), state: actor.clone() };
            let _record: Vec<Record> =
                engine.db().create(DB_TABLE_ACTOR).content(content).await.map_err(SystemActorError::from)?;

            // Create and return the actor context
            let ctx = ActorContext::new(engine.clone(), id.clone(), actor);
            Ok(ctx)
        }
    }

    // Start the actor
    fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), Self::Error>>;
}

/// Database record for an actor
#[derive(Clone, Debug, Serialize)]
pub struct ActorRecord<T: Actor> {
    id: RecordId,
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
    pub async fn start(&mut self) -> Result<(), T::Error> {
        let mut actor = self.actor.clone();
        actor.start(self).await?;
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

    /// Internal method to prepare and send a message
    fn prepare_and_send_message<T>(
        &self,
        message: T,
        to: &ActorId,
    ) -> impl Future<Output = Result<(RecordId, RecordId, Frame), SystemActorError>>
    where
        T: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync,
    {
        async move {
            let msg_value = serde_json::to_value(&message)?;
            let name = type_name::<T>();
            let msg_id = Id::ulid();
            let request_id = RecordId::from_table_key(DB_TABLE_MESSAGE, msg_id.to_string());
            let reply_id = RecordId::from_table_key(DB_TABLE_REPLY, msg_id.to_string());

            let request = Frame {
                id: request_id.clone(),
                name: name.into(),
                tx: self.id().id.clone(),
                rx: to.id.clone(),
                msg: msg_value.clone(),
            };

            debug!("[{}] msg-send {} {} {} {}", &self.id().id, name, &request.id, &to.id, &msg_value);

            let msg_engine = self.engine().clone();

            let task_request_id = request_id.clone();
            let task_request = request.clone();

            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(0)).await;
                let msg_ids: Result<Vec<Record>, _> =
                    msg_engine.db().create(DB_TABLE_MESSAGE).content(task_request).await;
                if let Ok(msg_ids) = msg_ids {
                    let id = msg_ids[0].id.clone();
                    if task_request_id != id {
                        error!("msg-send {}", &task_request_id);
                    }
                }
            });

            Ok((request_id, reply_id, request))
        }
    }

    fn do_send<M, T>(&self, message: T, to: &ActorId) -> impl Future<Output = Result<(), SystemActorError>>
    where
        M: Message<T>,
        T: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync,
    {
        async move {
            let _ = self.prepare_and_send_message(message, to).await?;
            Ok(())
        }
    }

    fn send<M, T>(&self, message: T, to: &ActorId) -> impl Future<Output = Result<M::Response, SystemActorError>>
    where
        M: Message<T>,
        T: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync,
    {
        async move {
            let (_, reply_id, _) = self.prepare_and_send_message(message, to).await?;
            self.wait_for_reply::<M::Response>(&reply_id).await
        }
    }

    fn do_send_as<M, R>(&self, message: M, to: &ActorId) -> impl Future<Output = Result<(), SystemActorError>>
    where
        M: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync,
        R: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync,
    {
        async move {
            let (_, _, _) = self.prepare_and_send_message(message, to).await?;
            Ok(())
        }
    }

    fn send_as<M, R>(&self, message: M, to: &ActorId) -> impl Future<Output = Result<R, SystemActorError>>
    where
        M: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync,
        R: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync,
    {
        async move {
            let (_, reply_id, _) = self.prepare_and_send_message(message, to).await?;
            self.wait_for_reply::<R>(&reply_id).await
        }
    }

    /// Wait for a reply to a sent message
    fn wait_for_reply<R>(&self, reply_id: &RecordId) -> impl Future<Output = Result<R, SystemActorError>>
    where
        R: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync,
    {
        async move {
            let mut stream = self.engine().db().select(reply_id).live().await?;
            let Some(notification) = stream.next().await else {
                return Err(SystemActorError::LiveStream("Empty stream".into()));
            };
            let notification: Notification<Frame> = notification?;
            let response = match notification.action {
                Action::Create => {
                    let data = &notification.data;
                    debug!("[{}] msg-done {} {} {} {}", &self.id().id, &data.name, &data.id, &data.rx, &data.msg);
                    Ok(data.msg.clone())
                }
                _ => Err(SystemActorError::LiveStream("Unexpected action".into())),
            }?;

            let response = serde_json::from_value(response)?;
            Ok(response)
        }
    }

    /// Reply to a received message
    fn reply<M, T>(&self, request: &Frame, message: M::Response) -> impl Future<Output = Result<(), SystemActorError>>
    where
        M: Message<T>,
        T: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static + Sync,
    {
        async move {
            let msg_value = serde_json::to_value(&message)?;

            // Use the request id as the reply id
            let reply_id = RecordId::from_table_key(DB_TABLE_REPLY, request.id.key().to_string());

            // Assert request.rx == self, can only reply to messages sent to us
            if request.rx != self.id().id {
                return Err(SystemActorError::IdMismatch(request.tx.clone(), self.id().id.clone()));
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
    fn recv(&self) -> impl Future<Output = Result<MessageStream, SystemActorError>> {
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
}

#[cfg(test)]
mod tests {
    use crate::dbg_export_db;
    use crate::prelude::*;
    use futures::StreamExt;
    use serde::{Deserialize, Serialize};
    use std::future::Future;
    use test_log::test;
    use tracing::{error, info};

    // Custom error types for PingActor and PongActor
    // Not neccesary, just for illustration purposes
    #[derive(Debug, thiserror::Error)]
    enum PingActorError {
        #[error("System error: {0}")]
        System(#[from] SystemActorError),
        #[error("Ping failed after {0} attempts")]
        PingFailed(usize),
    }

    impl ActorError for PingActorError {}

    #[derive(Debug, thiserror::Error)]
    enum PongActorError {
        #[error("System error: {0}")]
        System(#[from] SystemActorError),
        #[error("Pong limit reached")]
        PongLimitReached,
    }

    impl ActorError for PongActorError {}

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct Ping;

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct Pong {
        times: usize,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct PingActor {
        pong_id: ActorId,
        max_attempts: usize,
    }

    impl Actor for PingActor {
        type Error = PingActorError;

        fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), Self::Error>> {
            async move {
                info!("{} Says hi!", ctx.id());
                let pong_id = self.pong_id.clone();
                let mut attempts = 0;
                loop {
                    info!("{} Ping", ctx.id());
                    attempts += 1;
                    let pong = ctx.send::<PongActor, Ping>(Ping, &pong_id).await?;
                    info!("{} Pong {}", ctx.id(), pong.times);
                    if pong.times == 0 {
                        break;
                    }
                    if attempts >= self.max_attempts {
                        return Err(PingActorError::PingFailed(attempts));
                    }
                }
                info!("{} Says bye!", ctx.id());
                Ok(())
            }
        }
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct PongActor {
        times: usize,
    }

    impl Actor for PongActor {
        type Error = PongActorError;

        fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), Self::Error>> {
            async move {
                info!("{} Says hi!", ctx.id());
                let mut stream = ctx.recv().await?;
                while let Some(Ok(frame)) = stream.next().await {
                    if let Some(message) = frame.is::<Ping>() {
                        info!("{} Pong", ctx.id());
                        let _response = self.reply(ctx, &message, &frame).await?;
                        if self.times == 0 {
                            break;
                        }
                    }
                }
                info!("{} Says bye!", ctx.id());
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
        ) -> impl Future<Output = Result<Self::Response, Self::Error>> {
            async move {
                if self.times == 0 {
                    Err(PongActorError::PongLimitReached)
                } else {
                    self.times -= 1;
                    Ok(Pong { times: self.times })
                }
            }
        }
    }

    #[test(tokio::test)]
    async fn test_actor_ping_pong() -> Result<(), Box<dyn std::error::Error>> {
        let engine = Engine::test().await?;

        // Create actor_ids for the ping and pong actors
        let pong_id = ActorId::of::<PongActor>("/pong");
        let ping_id = ActorId::of::<PingActor>("/ping");

        // Spawn the ping and pong actors
        let mut ping_actor =
            Actor::spawn(&engine, &ping_id, PingActor { pong_id: pong_id.clone(), max_attempts: 5 }).await?;
        let mut pong_actor = Actor::spawn(&engine, &pong_id, PongActor { times: 3 }).await?;

        // Check health of the actors
        let ping_health = ping_actor.health().await;
        let pong_health = pong_actor.health().await;
        assert!(ping_health);
        assert!(pong_health);

        // Start the ping and pong actors
        let ping_handle = tokio::spawn(async move {
            if let Err(e) = ping_actor.start().await {
                error!("PingActor error: {}", e);
            }
        });
        let pong_handle = tokio::spawn(async move {
            if let Err(e) = pong_actor.start().await {
                error!("PongActor error: {}", e);
            }
        });

        // Wait for the ping and pong actors to finish
        let _ = ping_handle.await;
        let _ = pong_handle.await;

        // Export the database for debugging
        dbg_export_db!(engine);

        Ok(())
    }
}
