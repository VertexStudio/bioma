use crate::engine::{Engine, Record};
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use surrealdb::{sql::Id, value::RecordId, Action, Notification};
use tracing::{debug, error, trace};

// Constants for database table names
const DB_TABLE_ACTOR: &str = "actor";
const DB_TABLE_MESSAGE: &str = "message";
const DB_TABLE_REPLY: &str = "reply";

/// Implement this trait to define custom actor error types
pub trait ActorError: std::error::Error + Debug + Send + Sync + From<SystemActorError> {}

/// Enumerates the types of errors that can occur in Actor framework
#[derive(thiserror::Error, Debug)]
pub enum SystemActorError {
    // IO error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    // SurrealDB error
    #[error("Engine error: {0}")]
    EngineError(#[from] surrealdb::Error),
    // Message are not of the same type
    #[error("MessageTypeMismatch: {0}")]
    MessageTypeMismatch(&'static str),
    // LIVE query error
    #[error("LiveStream error: {0}")]
    LiveStream(Cow<'static, str>),
    // json_serde::Error
    #[error("JsonSerde error: {0}")]
    JsonSerde(#[from] serde_json::Error),
    // Id mismatch a and b
    #[error("Id mismatch: {0:?} {1:?}")]
    IdMismatch(RecordId, RecordId),
    #[error("Actor tag mismatch: {0} {1}")]
    ActorTagMismatch(Cow<'static, str>, Cow<'static, str>),
    // Actor already exists
    #[error("Actor already exists: {0}")]
    ActorAlreadyExists(ActorId),
    // Message reply error
    #[error("Message reply error: {0}")]
    MessageReply(Cow<'static, str>),
    #[error("Message timeout {0} {1:?}")]
    MessageTimeout(Cow<'static, str>, std::time::Duration),
    #[error("Tasked timeout after {0:?}")]
    TaskTimeout(std::time::Duration),
    #[error("Object store error: {0}")]
    ObjectStore(#[from] object_store::Error),
    #[error("Object store error: {0}")]
    PathError(#[from] object_store::path::Error),
    #[error("Url error: {0}")]
    UrlError(#[from] url::ParseError),
    #[error("Join handle error: {0}")]
    JoinHandle(#[from] tokio::task::JoinError),
    #[error("Actor tag already registered: {0}")]
    ActorTagAlreadyRegistered(Cow<'static, str>),
    #[error("Actor tag not found: {0}")]
    ActorTagNotFound(Cow<'static, str>),
}

impl ActorError for SystemActorError {}

/// The message frame that is sent between actors
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrameMessage {
    /// Message id created with a ULID
    id: RecordId,
    /// Message name (usually a type name)
    pub name: Cow<'static, str>,
    /// Sender
    pub tx: RecordId,
    /// Receiver
    pub rx: RecordId,
    /// Message content
    #[serde(default)]
    pub msg: Value,
}

impl FrameMessage {
    /// Check if this frame matches a specific message type
    /// and deserialize it into the message type.
    ///
    /// # Type Parameters
    ///
    /// * M - The message type to check against and deserialize into.
    ///
    /// # Returns
    ///
    /// * Some(M) if the frame's name matches the type name of M and deserialization succeeds.
    /// * None if the frame's name doesn't match or deserialization fails.
    pub fn is<M>(&self) -> Option<M>
    where
        M: Clone + Serialize + for<'de> Deserialize<'de> + Send + Sync,
    {
        if self.name == std::any::type_name::<M>() {
            serde_json::from_value(self.msg.clone()).ok()
        } else {
            None
        }
    }
}

/// The reply frame that is sent as a response to a message
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrameReply {
    /// Message id created with a ULID or structured JSON for chunking
    ///
    /// The structure of `id` determines if it's a chunked reply or a final reply:
    /// - For partial/chunked replies: `reply:{id: "some_base_id", chunk: N}`
    /// - For the final reply: `reply:{id: "some_base_id"}`
    ///
    /// If the `id` contains a "chunk" field, it's a partial reply.
    /// If not, it's the final reply.
    id: RecordId,
    /// Message name (usually a type name)
    pub name: Cow<'static, str>,
    /// Sender (the actor replying)
    pub tx: RecordId,
    /// Receiver (the actor who sent the original message)
    pub rx: RecordId,
    /// Message content
    #[serde(default)]
    pub msg: Value,
    /// Error message if any
    #[serde(default)]
    pub err: Value,
}

pub type MessageStream = Pin<Box<dyn Stream<Item = Result<FrameMessage, SystemActorError>> + Send>>;

/// A trait for message types that can be sent between actors.
pub trait MessageType: Clone + Serialize + for<'de> Deserialize<'de> + Send + Sync {}
// Blanket implementation for all types that meet the criteria
impl<T> MessageType for T where T: Clone + Serialize + for<'de> Deserialize<'de> + Send + Sync {}

/// Defines message handling behavior for actors.
///
/// This trait should be implemented by actors that want to handle specific message types.
/// It provides methods for processing incoming messages and generating responses.
///
/// # Type Parameters
///
/// * MT: The specific message type this implementation handles.
pub trait Message<MT>: Actor
where
    MT: MessageType,
{
    type Response: Clone + Serialize + for<'de> Deserialize<'de> + Send + Sync;

    /// Handles a message of type MT for this actor.
    ///
    /// This function must be implemented for any actor that wants to handle messages of type MT.
    /// It defines how the actor processes the incoming message and what response it generates.
    ///
    /// # Arguments
    ///
    /// * self - A mutable reference to the actor instance.
    /// * ctx - A mutable reference to the actor's context, providing access to the actor's state and environment.
    /// * message - A reference to the message of type MT to be handled.
    ///
    /// # Returns
    ///
    /// An implementation of Future that resolves to a Result containing either:
    /// - Ok(Self::Response): The successful response to the message.
    /// - Err(Self::Error): An error that occurred during message handling.
    fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: &MT,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>>;

    /// Handle and reply to a message.
    ///
    /// This method will:
    /// 1. Call the actor's handle method for this message type
    /// 2. Reply to the sender of the message with the response
    ///
    /// By default, this sends a single final reply (no chunk in RecordId).
    /// If you want to send multiple partial replies, call `ctx.reply_chunked` multiple
    /// times with different RecordIds that include a `chunk` field until the final one
    /// without `chunk`.
    ///
    /// # Arguments
    ///
    /// * ctx - A mutable reference to the actor context
    /// * message - A reference to the message to be handled
    /// * frame - A reference to the original message frame
    ///
    /// # Returns
    ///
    /// A future that resolves to a Result containing the response or an error
    fn reply(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: &MT,
        frame: &FrameMessage,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> {
        async move {
            let response = self.handle(ctx, message).await;
            // Final single reply without chunk
            ctx.reply_chunked::<Self, MT>(frame, &response, None).await?;
            response
        }
    }

    /// Send a message to an actor without waiting for a reply
    ///
    /// This sends a message and does not block waiting for any reply.
    fn do_send(
        &self,
        ctx: &mut ActorContext<Self>,
        message: MT,
        to: &ActorId,
    ) -> impl Future<Output = Result<(), SystemActorError>> {
        async move {
            let _ = ctx.prepare_and_send_message::<MT>(&message, to).await?;
            Ok(())
        }
    }

    /// Send a message to an actor and wait for a final reply
    ///
    /// # Arguments
    ///
    /// * ctx - A mutable reference to the actor context
    /// * message - The message to send
    /// * to - The ActorId of the recipient
    /// * options - SendOptions for timeout configuration
    ///
    /// # Returns
    ///
    /// A Result with the final response or a timeout/error.
    fn send(
        &self,
        ctx: &mut ActorContext<Self>,
        message: MT,
        to: &ActorId,
        options: SendOptions,
    ) -> impl Future<Output = Result<Self::Response, SystemActorError>> {
        async move {
            let (_, reply_id, _) = ctx.prepare_and_send_message::<MT>(&message, to).await?;
            ctx.wait_for_reply::<Self::Response>(&reply_id, options).await
        }
    }
}

/// Options for configuring message sending behavior.
#[derive(bon::Builder)]
pub struct SendOptions {
    /// The maximum duration to wait for a reply before timing out.
    pub timeout: std::time::Duration,
}

impl Default for SendOptions {
    fn default() -> Self {
        Self { timeout: std::time::Duration::from_secs(30) }
    }
}

/// A unique identifier for a distributed actor
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Hash, Eq)]
pub struct ActorId {
    name: Cow<'static, str>,
    tag: Cow<'static, str>,
}

impl std::fmt::Display for ActorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", &self.name, &self.tag)
    }
}

impl ActorId {
    /// Creates an actor id to reference a specific actor.
    ///
    /// This method generates a unique ActorId for a given actor type and identifier.
    pub fn of<T: Actor>(uid: impl Into<Cow<'static, str>>) -> Self {
        let name = uid.into();
        Self { tag: std::any::type_name::<T>().into(), name }
    }

    pub fn with_tag(name: impl Into<Cow<'static, str>>, tag: impl Into<Cow<'static, str>>) -> Self {
        Self { tag: tag.into(), name: name.into() }
    }

    pub fn record_id(&self) -> RecordId {
        RecordId::from_table_key(DB_TABLE_ACTOR, self.name.as_ref())
    }

    pub fn tag(&self) -> &str {
        &self.tag
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Options for spawning an actor.
///
/// Configuration options for the actor spawning process.
#[derive(bon::Builder, Clone)]
pub struct SpawnOptions {
    /// Specifies how to handle the case when an actor with the same ID already exists.
    ///
    /// This field determines the behavior of the spawning process when it encounters
    /// an existing actor with the same ID as the one being spawned.
    exists: SpawnExistsOptions,
}

impl Default for SpawnOptions {
    fn default() -> Self {
        Self { exists: SpawnExistsOptions::Error }
    }
}

/// Options for handling existing actors during spawn.
#[derive(Clone)]
pub enum SpawnExistsOptions {
    /// Reset the actor if it already exists.
    ///
    /// This option will delete the existing actor's record and create a new one.
    Reset,

    /// Error if the actor already exists.
    ///
    /// This option will return an error if an actor with the same ID already exists.
    Error,

    /// Restore the actor if it already exists.
    ///
    /// This option will load the existing actor's state from the database and use it
    /// instead of creating a new actor instance.
    Restore,
}

/// Implement this trait to define an actor
pub trait Actor: Sized + Serialize + for<'de> Deserialize<'de> + Debug + Send + Sync {
    type Error: ActorError;

    /// Spawns a new actor in the system or handles an existing one based on the provided options.
    ///
    /// This function creates a new actor instance, registers it in the database, or restores an existing actor.
    ///
    /// # Arguments
    ///
    /// * `engine` - The `Engine` instance.
    /// * `id` - The `ActorId` for the actor.
    /// * `actor` - The actor instance to be spawned.
    /// * `options` - `SpawnOptions` to control behavior when the actor already exists.
    ///
    /// # Returns
    ///
    /// A `Future` that resolves to a `Result` containing either:
    /// - `Ok((ActorContext<Self>, Self))`: The actor context and the actor instance.
    /// - `Err(Self::Error)`: An error if the spawning process fails.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - The actor tag in the `id` doesn't match the actual actor type.
    /// - Serialization of the actor state fails.
    /// - Creating or updating the actor record in the database fails.
    /// - The actor already exists and `SpawnOptions::Error` is specified.
    /// - Deserialization of an existing actor's state fails when using `SpawnOptions::Restore`.
    fn spawn(
        engine: Engine,
        id: ActorId,
        actor: Self,
        options: SpawnOptions,
    ) -> impl Future<Output = Result<(ActorContext<Self>, Self), Self::Error>> {
        async move {
            // Check if the actor already exists
            let actor_record: Option<ActorRecord> =
                engine.db().select(&id.record_id()).await.map_err(SystemActorError::from)?;

            if let Some(actor_record) = actor_record {
                // Actor exists, apply options
                match options.exists {
                    SpawnExistsOptions::Reset => {
                        // Reset the actor by deleting its record
                        let _: Option<ActorRecord> =
                            engine.db().delete(&id.record_id()).await.map_err(SystemActorError::from)?;
                        // We'll create a new record below
                    }
                    SpawnExistsOptions::Error => {
                        return Err(Self::Error::from(SystemActorError::ActorAlreadyExists(id.clone())));
                    }
                    SpawnExistsOptions::Restore => {
                        // Restore the actor by loading its state from the database
                        let actor_state = actor_record.state;
                        let actor: Self = serde_json::from_value(actor_state).map_err(SystemActorError::from)?;
                        // Create and return the actor context with restored state
                        let ctx =
                            ActorContext { engine: engine.clone(), id: id.clone(), _marker: std::marker::PhantomData };
                        return Ok((ctx, actor));
                    }
                }
            }

            // At this point, we either need to create a new actor or reset an existing one

            // Serialize actor properties
            let actor_state = serde_json::to_value(&actor).map_err(SystemActorError::from)?;

            // Create or update actor record in the database
            let content = ActorRecord { id: id.record_id(), tag: id.tag.clone(), state: actor_state };
            let _record: Option<Record> =
                engine.db().create(DB_TABLE_ACTOR).content(content).await.map_err(SystemActorError::from)?;

            // Create and return the actor context
            let ctx = ActorContext { engine: engine.clone(), id: id.clone(), _marker: std::marker::PhantomData };
            Ok((ctx, actor))
        }
    }

    /// Starts the actor's main loop.
    ///
    /// This function is the entry point for the actor's lifecycle.
    ///
    /// # Arguments
    ///
    /// * &mut self - A mutable reference to the actor instance.
    /// * ctx - A mutable reference to the actor's context, providing access to the actor's
    ///   state and communication methods.
    ///
    /// # Returns
    ///
    /// Returns a Result<(), Self::Error>:
    /// - Ok(()) if the actor completes its work successfully.
    /// - Err(Self::Error) if an error occurs during the actor's execution.
    fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), Self::Error>>;

    /// Saves the current state of the actor in the system.
    ///
    /// This function updates the actor's state in the database.
    ///
    /// # Arguments
    ///
    /// * ctx - A reference to the ActorContext instance.
    ///
    /// # Returns
    ///
    /// A Future that resolves to a Result containing either:
    /// - Ok(()): The actor state was successfully saved.
    /// - Err(Self::Error): An error if the saving process fails.
    fn save(&self, ctx: &ActorContext<Self>) -> impl Future<Output = Result<(), Self::Error>> {
        async move {
            // Serialize actor properties
            let actor_state = serde_json::to_value(self).map_err(SystemActorError::from)?;

            let record_id = ctx.id().record_id();

            // Update actor record in the database
            let content = ActorRecord { id: record_id.clone(), tag: ctx.id().tag.clone(), state: actor_state };

            let _record: Option<Record> =
                ctx.engine().db().update(&record_id).content(content).await.map_err(SystemActorError::from)?;

            Ok(())
        }
    }
}

/// Database record for an actor
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActorRecord {
    id: RecordId,
    tag: Cow<'static, str>,
    #[serde(default)]
    state: Value,
}

/// Context for an actor, that binds an actor to its engine
#[derive(Debug)]
pub struct ActorContext<T: Actor> {
    engine: Engine,
    id: ActorId,
    _marker: std::marker::PhantomData<T>,
}

impl<T: Actor> ActorContext<T> {
    async fn unreplied_messages(&self) -> Result<Vec<FrameMessage>, SystemActorError> {
        let query = include_str!("../sql/unreplied_messages.surql");
        let mut res = self.engine().db().query(query).bind(("rx", self.id.record_id())).await?;
        let existing_messages: Vec<FrameMessage> = res.take(0)?;
        trace!("unreplied_messages: {:?}", existing_messages);
        Ok(existing_messages)
    }

    /// Get the actor id
    pub fn id(&self) -> &ActorId {
        &self.id
    }

    /// Get the engine
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Check the health of the actor
    pub async fn health(&self) -> bool {
        // Check if the actor is still in the database
        let record: Result<Option<ActorRecord>, SystemActorError> =
            self.engine().db().select(&self.id.record_id()).await.map_err(SystemActorError::from);
        if let Ok(Some(_)) = record {
            true
        } else {
            false
        }
    }

    /// Kill the actor
    pub async fn kill(&self) -> Result<(), SystemActorError> {
        let _: Option<ActorRecord> =
            self.engine().db().delete(&self.id.record_id()).await.map_err(SystemActorError::from)?;
        Ok(())
    }

    /// Internal method to prepare and send a message
    async fn prepare_and_send_message<MT>(
        &self,
        message: &MT,
        to: &ActorId,
    ) -> Result<(RecordId, RecordId, FrameMessage), SystemActorError>
    where
        MT: MessageType,
    {
        let msg_value = serde_json::to_value(&message)?;
        let name = std::any::type_name::<MT>();
        let msg_id = Id::ulid();
        let request_id = RecordId::from_table_key(DB_TABLE_MESSAGE, msg_id.to_string());
        let reply_id = RecordId::from_table_key(DB_TABLE_REPLY, msg_id.to_string());

        let request = FrameMessage {
            id: request_id.clone(),
            name: name.into(),
            tx: self.id().record_id(),
            rx: to.record_id(),
            msg: msg_value.clone(),
        };

        debug!("[{}] msg-send {} {} {} {}", &self.id().record_id(), name, &request.id, &to.record_id(), &msg_value);

        let db = self.engine().db().clone();

        let task_request_id = request_id.clone();
        let task_request = request.clone();

        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(0)).await;
            let msg_id: Result<Option<Record>, surrealdb::Error> =
                db.create(DB_TABLE_MESSAGE).content(task_request).await;
            if let Ok(Some(msg_id)) = msg_id {
                let id = msg_id.id.clone();
                if task_request_id != id {
                    error!("msg-send {}", &task_request_id);
                }
            } else {
                error!("msg-send {:?}", msg_id);
            }
        });

        Ok((request_id, reply_id, request))
    }

    /// Send a message to an actor without waiting for a reply.
    ///
    /// This method sends a message to another actor without expecting or waiting for a response.
    /// It's useful for fire-and-forget type operations where you don't need a reply.
    pub async fn do_send<M, MT>(&self, message: MT, to: &ActorId) -> Result<(), SystemActorError>
    where
        M: Message<MT>,
        MT: MessageType,
    {
        let _ = self.prepare_and_send_message::<MT>(&message, to).await?;
        Ok(())
    }

    /// Send a message to an actor and wait for a final reply
    ///
    /// # Returns
    ///
    /// Ok(M::Response) if final reply is received, Err(...) otherwise.
    pub async fn send<M, MT>(
        &self,
        message: MT,
        to: &ActorId,
        options: SendOptions,
    ) -> Result<M::Response, SystemActorError>
    where
        M: Message<MT>,
        MT: MessageType,
    {
        let (_, reply_id, _) = self.prepare_and_send_message::<MT>(&message, to).await?;
        self.wait_for_reply::<M::Response>(&reply_id, options).await
    }

    /// Send a message to an actor without waiting for a reply.
    ///
    /// This method allows sending a message without expecting a response.
    pub async fn do_send_as<MT>(&self, message: MT, to: &ActorId) -> Result<(), SystemActorError>
    where
        MT: MessageType,
    {
        let (_, _, _) = self.prepare_and_send_message(&message, to).await?;
        Ok(())
    }

    /// Send a message to an actor and wait for a reply of a specific type RT.
    ///
    /// This is similar to `send` but allows for different request and response types.
    ///
    /// Waits for the final reply.
    pub async fn send_as<MT, RT>(&self, message: MT, to: ActorId, options: SendOptions) -> Result<RT, SystemActorError>
    where
        MT: MessageType,
        RT: MessageType,
    {
        let (_, reply_id, _) = self.prepare_and_send_message::<MT>(&message, &to).await?;
        self.wait_for_reply::<RT>(&reply_id, options).await
    }

    /// Determine if a given `FrameReply.id` is a chunked reply or a final reply.
    ///
    /// If the underlying ID is a SurrealDB object containing a "chunk" field, it's partial.
    /// If not, it's final.
    fn is_chunked_reply(reply_id: &RecordId) -> bool {
        // reply_id.id can be a string, number or object.
        // For chunked replies we assume: reply:{"id": "someid", "chunk": number}
        // For final replies we assume: reply:{"id": "someid"}
        //
        // Let's check if "chunk" exists in the object.
        match reply_id.key() {
            RecordIdKey::Object(obj) => {
                // Check if obj has a key "chunk"
                obj.contains_key("chunk")
            }
            _ => false,
        }
    }

    /// Wait for a final reply to a sent message
    ///
    /// This method will consume replies until it finds one without a `chunk` field in its ID,
    /// which is considered the final reply. All intermediate replies with `chunk` fields are
    /// partial (chunked) replies and are ignored here.
    async fn wait_for_reply<RT>(&self, reply_id: &RecordId, options: SendOptions) -> Result<RT, SystemActorError>
    where
        RT: MessageType,
    {
        let mut stream = self.engine().db().select(reply_id).live().await?;
        let start = std::time::Instant::now();

        loop {
            let elapsed = start.elapsed();
            if elapsed > options.timeout {
                return Err(SystemActorError::MessageTimeout(std::any::type_name::<RT>().into(), options.timeout));
            }

            let delay = options.timeout.saturating_sub(elapsed);
            let notification = tokio::time::timeout(delay, stream.next())
                .await
                .map_err(|_| SystemActorError::MessageTimeout(std::any::type_name::<RT>().into(), options.timeout))?;

            let notification = notification.ok_or_else(|| SystemActorError::LiveStream("Empty stream".into()))?;
            let notification: Notification<FrameReply> = notification?;

            match notification.action {
                Action::Create => {
                    let data = &notification.data;
                    debug!(
                        "[{}] msg-done {} {} {} {:?}",
                        &self.id().record_id(),
                        &data.name,
                        &data.id,
                        &data.rx,
                        &data.msg
                    );

                    if !data.err.is_null() {
                        return Err(SystemActorError::MessageReply(data.err.to_string().into()));
                    }

                    // Check if final reply (no chunk) or partial reply (chunk)
                    if Self::is_chunked_reply(&data.id) {
                        // Partial reply with chunk, ignore and continue
                        continue;
                    } else {
                        // Final reply
                        let response = serde_json::from_value::<RT>(data.msg.clone())?;
                        return Ok(response);
                    }
                }
                _ => {
                    return Err(SystemActorError::LiveStream("Unexpected action".into()));
                }
            }
        }
    }

    /// Reply to a received message, either partially or finally, by constructing a new RecordId.
    ///
    /// If `chunk` is `Some(...)`, we create a RecordId with a chunk field:
    /// `reply:{"id":"original_msg_id","chunk":N}`.
    ///
    /// If `chunk` is `None`, we create a final reply id:
    /// `reply:{"id":"original_msg_id"}` (no chunk field).
    ///
    /// # Arguments
    ///
    /// * request: A reference to the original FrameMessage that is being replied to.
    /// * message: A reference to the Result containing either the response or an error.
    /// * chunk: Some(chunk_number) for partial replies, None for final reply.
    ///
    /// # Returns
    ///
    /// A Result which is:
    /// - Ok(()) if the reply was successfully sent.
    /// - Err(SystemActorError) if there was an error in sending the reply.
    pub async fn reply_chunked<M, MT>(
        &self,
        request: &FrameMessage,
        message: &Result<M::Response, T::Error>,
        chunk: Option<usize>,
    ) -> Result<(), SystemActorError>
    where
        M: Message<MT>,
        MT: MessageType,
    {
        let msg_value = match message {
            Ok(msg) => serde_json::to_value(&msg)?,
            Err(err) => serde_json::to_value(&err.to_string())?,
        };

        // Construct a reply_id based on whether chunk is provided or not
        let base_id = request.id.key().to_string();
        let reply_id = if let Some(c) = chunk {
            RecordId::from((DB_TABLE_REPLY, serde_json::json!({"id": base_id, "chunk": c})))
        } else {
            // Final reply
            RecordId::from((DB_TABLE_REPLY, serde_json::json!({"id": base_id})))
        };

        // Ensure we're replying to a message intended for us
        if request.rx != self.id().record_id() {
            return Err(SystemActorError::IdMismatch(request.tx.clone(), self.id().record_id()));
        }

        let (msg_val, err_val) = match message {
            Ok(_) => (msg_value.clone(), serde_json::Value::Null),
            Err(_) => (serde_json::Value::Null, msg_value.clone()),
        };

        let reply = FrameReply {
            id: reply_id.clone(),
            name: std::any::type_name::<M::Response>().into(),
            tx: request.rx.clone(),
            rx: request.tx.clone(),
            msg: msg_val,
            err: err_val,
        };

        debug!("[{}] msg-rply {} {} {} {:?}", &self.id().record_id(), &reply.name, &reply_id, &reply.tx, &reply.msg);

        let reply_query = include_str!("../sql/reply.surql");

        let response = self
            .engine()
            .db()
            .query(reply_query)
            .bind(("reply_id", reply_id))
            .bind(("reply", reply))
            .bind(("msg_id", request.id.clone()))
            .await?;
        let response = response.check();
        if let Err(e) = response {
            error!("msg-rply {:?}", e);
        }
        Ok(())
    }

    /// Receive messages for this actor
    ///
    /// This method sets up a stream of messages for the actor, combining any unreplied messages
    /// with a live query for new incoming messages.
    ///
    /// # Returns
    ///
    /// Returns a Result containing:
    /// - Ok(MessageStream): A pinned box containing a stream of Result<FrameMessage, SystemActorError>.
    /// - Err(SystemActorError): If there's an error setting up the message stream.
    pub async fn recv(&self) -> Result<MessageStream, SystemActorError> {
        let query = format!("LIVE SELECT * FROM {} WHERE rx = {}", DB_TABLE_MESSAGE, self.id().record_id());
        debug!("[{}] msg-live {}", &self.id().record_id(), &query);
        let mut res = self.engine().db().query(&query).await?;
        let live_query = res.stream::<Notification<FrameMessage>>(0)?;
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
                        &self_id.record_id(),
                        &frame.name,
                        &frame.id,
                        &frame.tx,
                        &frame.rx,
                        &frame.msg
                    );
                }
                Err(error) => debug!("msg-recv {} {:?}", self_id.record_id(), error),
            });

        let unreplied_messages = self.unreplied_messages().await?;
        let unreplied_stream = futures::stream::iter(unreplied_messages).map(Ok);
        let chained_stream = unreplied_stream.chain(live_query);

        Ok(Box::pin(chained_stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dbg_export_db;
    use crate::prelude::*;
    use futures::StreamExt;
    use serde::{Deserialize, Serialize};
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

    #[derive(Debug, Serialize, Deserialize)]
    struct PingActor {
        pong_id: ActorId,
        max_attempts: usize,
    }

    impl Actor for PingActor {
        type Error = PingActorError;

        async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
            info!("{} Says hi!", ctx.id());
            let pong_id = self.pong_id.clone();
            let mut attempts = 0;
            loop {
                info!("{} Ping", ctx.id());
                attempts += 1;
                let pong = ctx.send::<PongActor, Ping>(Ping, &pong_id, SendOptions::default()).await?;
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

    #[derive(Debug, Serialize, Deserialize)]
    struct PongActor {
        times: usize,
    }

    impl Actor for PongActor {
        type Error = PongActorError;

        async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
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

    impl Message<Ping> for PongActor {
        type Response = Pong;

        async fn handle(
            &mut self,
            _ctx: &mut ActorContext<Self>,
            _message: &Ping,
        ) -> Result<Self::Response, Self::Error> {
            if self.times == 0 {
                Err(PongActorError::PongLimitReached)
            } else {
                self.times -= 1;
                Ok(Pong { times: self.times })
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
        let (mut ping_ctx, mut ping_actor) = Actor::spawn(
            engine.clone(),
            ping_id.clone(),
            PingActor { pong_id: pong_id.clone(), max_attempts: 5 },
            SpawnOptions::default(),
        )
        .await?;
        let (mut pong_ctx, mut pong_actor) =
            Actor::spawn(engine.clone(), pong_id.clone(), PongActor { times: 3 }, SpawnOptions::default()).await?;

        // Check health of the actors
        let ping_health = ping_ctx.health().await;
        let pong_health = pong_ctx.health().await;
        assert!(ping_health);
        assert!(pong_health);

        // Start the ping and pong actors
        let ping_handle = tokio::spawn(async move {
            if let Err(e) = ping_actor.start(&mut ping_ctx).await {
                error!("PingActor error: {}", e);
            }
        });
        let pong_handle = tokio::spawn(async move {
            if let Err(e) = pong_actor.start(&mut pong_ctx).await {
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
