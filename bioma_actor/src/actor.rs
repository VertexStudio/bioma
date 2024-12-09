use crate::engine::{Engine, Record};
use futures::{future, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use surrealdb::RecordIdKey;
use tokio::sync::mpsc;
// use std::any::type_name;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::{borrow::Cow, sync::atomic::AtomicU64};
use surrealdb::{sql::Id, value::RecordId, Action, Notification};
use tokio::time::timeout;
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
    /// * `M` - The message type to check against and deserialize into.
    ///
    /// # Returns
    ///
    /// * `Some(M)` if the frame's name matches the type name of `M` and deserialization succeeds.
    /// * `None` if the frame's name doesn't match or deserialization fails.
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

/// Identifier for a reply message that may be part of a stream.
///
/// Reply IDs contain both a base message ID and an optional chunk number
/// to support streaming replies.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplyId {
    /// Base message identifier
    id: String,
    /// Optional chunk number for streaming replies.
    /// - `Some(n)` indicates this is chunk `n` of an ongoing stream
    /// - `None` indicates this is the final reply in the stream
    chunk: Option<u64>,
}

impl ReplyId {
    /// Creates a new ReplyId for a chunk in a stream
    pub fn new_chunk(id: String, chunk: u64) -> Self {
        Self { id, chunk: Some(chunk) }
    }

    /// Creates a new ReplyId for a final reply
    pub fn new_final(id: String) -> Self {
        Self { id, chunk: None }
    }

    /// Converts the ReplyId to a SurrealDB record ID
    pub fn to_record_id(&self) -> surrealdb::RecordId {
        let mut obj = surrealdb::Object::new();
        obj.insert(
            "id".to_string(),
            surrealdb::Value::from_inner(surrealdb::sql::Value::Strand(surrealdb::sql::Strand::from(self.id.clone()))),
        );

        obj.insert(
            "chunk".to_string(),
            match self.chunk {
                Some(chunk) => surrealdb::Value::from_inner(surrealdb::sql::Value::Number(
                    surrealdb::sql::Number::Int(chunk as i64),
                )),
                None => surrealdb::Value::from_inner(surrealdb::sql::Value::None),
            },
        );

        let key = RecordIdKey::from(obj);
        surrealdb::RecordId::from_table_key(DB_TABLE_REPLY, key)
    }
}

/// A frame containing a reply message from an actor.
///
/// Reply frames can represent either:
/// - A chunk of an ongoing reply stream (when `id.chunk` is `Some`)
/// - The final reply in a stream (when `id.chunk` is `None`)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrameReply {
    /// Reply identifier containing the message ID and optional chunk number
    id: ReplyId,
    /// Message name (usually a type name)
    pub name: Cow<'static, str>,
    /// Sender
    pub tx: RecordId,
    /// Receiver
    pub rx: RecordId,
    /// Message content
    #[serde(default)]
    pub msg: Value,
    /// Error message
    #[serde(default)]
    pub err: Value,
}

impl FrameReply {
    /// Creates a new reply frame for a chunk in a streaming response
    pub fn new_chunk(
        id: String,
        chunk_num: u64,
        name: Cow<'static, str>,
        tx: surrealdb::RecordId,
        rx: surrealdb::RecordId,
        msg: Value,
    ) -> Self {
        Self { id: ReplyId::new_chunk(id, chunk_num), name, tx, rx, msg, err: Value::Null }
    }

    /// Creates a new reply frame for a final response
    pub fn new_final(id: String, name: Cow<'static, str>, tx: surrealdb::RecordId, rx: surrealdb::RecordId) -> Self {
        Self { id: ReplyId::new_final(id), name, tx, rx, msg: Value::Null, err: Value::Null }
    }
}

/// A stream of replies from an actor in response to a message.
///
/// This type represents an asynchronous stream of responses that can be consumed
/// one at a time. The stream will continue until either:
/// - A final reply is received (indicated by `chunk = None`)
/// - An error occurs
/// - The stream times out
pub type ReplyStream<T> = Pin<Box<dyn Stream<Item = Result<T, SystemActorError>> + Send>>;

/// Type representing a stream of messages to an actor.
///
/// A MessageStream provides an ordered sequence of incoming messages that can be
/// processed asynchronously. It combines:
/// - Existing unreplied messages from the database
/// - New messages as they arrive
pub type MessageStream = Pin<Box<dyn Stream<Item = Result<FrameMessage, SystemActorError>> + Send>>;

/// A trait for types that can be sent as messages between actors.
///
/// This trait is automatically implemented for types that meet the requirements:
/// - Clone for message passing
/// - Serialize/Deserialize for transport
/// - Send + Sync for thread safety
///
/// # Example
///
/// ```rust
/// #[derive(Clone, Serialize, Deserialize)]
/// struct MyMessage {
///     data: String,
///     timestamp: u64,
/// }
///
/// // MyMessage automatically implements MessageType
/// ```
pub trait MessageType: Clone + Serialize + for<'de> Deserialize<'de> + Send + Sync {}
// Blanket implementation for all types that meet the criteria
impl<T> MessageType for T where T: Clone + Serialize + for<'de> Deserialize<'de> + Send + Sync {}

/// A trait for actors that can handle specific message types.
///
/// This trait should be implemented by actors that want to handle messages of type `MT`.
/// The implementation defines how the actor processes messages and sends responses
/// through its context.
///
/// # Type Parameters
///
/// * `MT`: The specific message type this implementation handles
///
/// # Examples
///
/// ```rust
/// struct MyActor {
///     counter: usize,
/// }
///
/// #[derive(Clone, Serialize, Deserialize)]
/// struct IncrementMessage(usize);
///
/// #[derive(Clone, Serialize, Deserialize)]
/// struct CounterResponse {
///     previous: usize,
///     current: usize,
/// }
///
/// impl Message<IncrementMessage> for MyActor {
///     type Response = CounterResponse;
///
///     async fn handle(
///         &mut self,
///         ctx: &mut ActorContext<Self>,
///         message: &IncrementMessage
///     ) -> Result<(), Self::Error> {
///         let previous = self.counter;
///         self.counter += message.0;
///         
///         // Send the response through the context
///         ctx.reply(CounterResponse {
///             previous,
///             current: self.counter
///         }).await?;
///         
///         Ok(())
///     }
/// }
/// ```
pub trait Message<MT>: Actor
where
    MT: MessageType,
    Self::Response: 'static,
{
    /// The type of response this message handler produces.
    type Response: Clone + Serialize + for<'de> Deserialize<'de> + Send + Sync;

    /// Handles an incoming message of type `MT`.
    ///
    /// This method processes the message and can send multiple responses using
    /// the context's `reply()` method before completing.
    ///
    /// # Arguments
    ///
    /// * `self` - A mutable reference to the actor instance
    /// * `ctx` - The actor's context, used for sending replies
    /// * `message` - The message to process
    ///
    /// # Returns
    ///
    /// Returns a `Result` which is:
    /// - `Ok(())` if message processing completed successfully
    /// - `Err(Self::Error)` if an error occurred during processing
    fn handle(&mut self, ctx: &mut ActorContext<Self>, message: &MT) -> impl Future<Output = Result<(), Self::Error>>;

    /// Processes a message and manages the reply stream lifecycle.
    ///
    /// This method:
    /// 1. Sets up the reply stream
    /// 2. Calls `handle()` to process the message
    /// 3. Cleans up the reply stream
    /// 4. Ensures the final reply is sent
    ///
    /// Typically, you won't need to override this method as the default
    /// implementation handles the message processing lifecycle.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The actor's context
    /// * `message` - The message to process
    /// * `frame` - The original message frame
    ///
    /// # Returns
    ///
    /// Returns a `Result` which is:
    /// - `Ok(())` if the message was processed and all replies sent successfully
    /// - `Err(Self::Error)` if an error occurred during processing
    fn reply(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: &MT,
        frame: &FrameMessage,
    ) -> impl Future<Output = Result<(), Self::Error>> {
        async move {
            // Set up reply stream first
            let handle = ctx.start_message_processing(frame.clone()).await;

            // Process message and store result
            let result = self.handle(ctx, message).await;

            // Ensure cleanup happens regardless of handle result
            let cleanup_result = {
                // Clean up reply stream first
                ctx.finish_message_processing().await;

                // Then wait for final reply to be sent
                handle.await
            };

            if let Err(e) = cleanup_result {
                error!("Error during reply cleanup: {}", e);
            }

            result
        }
    }

    /// Send a message to an actor without waiting for a reply
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

    /// Send a message to an actor and wait for a reply
    fn send(
        &self,
        ctx: &mut ActorContext<Self>,
        message: MT,
        to: &ActorId,
        options: SendOptions,
    ) -> impl Future<Output = Result<ReplyStream<Self::Response>, SystemActorError>>
    where
        Self::Response: 'static,
    {
        async move {
            let (_, reply_id, _) = ctx.prepare_and_send_message::<MT>(&message, to).await?;
            ctx.wait_for_replies::<Self::Response>(&reply_id, options).await
        }
    }
}

/// Configuration options for message sending behavior.
///
/// Controls aspects of message delivery and reply handling such as:
/// - Timeout duration
///
/// # Example
///
/// ```rust
/// // Custom timeout for important message
/// let options = SendOptions::builder()
///     .timeout(std::time::Duration::from_secs(60))
///     .build();
///
/// // Send with custom options
/// let reply = ctx.send::<TargetActor, MyMessage>(
///     message,
///     &target_id,
///     options
/// ).await?;
/// ```
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

/// A unique identifier for an actor in the system.
///
/// Actor IDs combine:
/// - A unique name within namespace
/// - The actor's type tag
///
/// This ensures each actor has a globally unique identifier that
/// also carries type information.
///
/// # Examples
///
/// ```rust
/// // Create ID for specific actor type
/// let id = ActorId::of::<MyActor>("/my-actor-1");
///
/// // Create ID with custom type tag
/// let id = ActorId::with_tag("/my-actor-2", "custom.actor.type");
/// ```
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Hash, Eq)]
pub struct ActorId {
    /// Unique name within namespace
    name: Cow<'static, str>,
    /// Actor type identifier
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
    /// This method generates a unique `ActorId` for a given actor type and identifier.
    ///
    /// # Type Parameters
    ///
    /// * `T`: The type of the actor, which must implement the `Actor` trait.
    ///
    /// # Arguments
    ///
    /// * `uid`: A unique identifier for the actor, which can be any type that can be converted into a `Cow<'static, str>`.
    ///
    /// # Returns
    ///
    /// Returns a new `ActorId` instance with the generated id and the type name of the actor.
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

/// Options for configuring actor spawn behavior.
///
/// These options control how the system handles cases where an actor
/// with the same ID already exists when spawning.
///
/// # Examples
///
/// ```rust
/// // Error if actor exists
/// let options = SpawnOptions::default();
///
/// // Reset existing actor
/// let options = SpawnOptions::builder()
///     .exists(SpawnExistsOptions::Reset)
///     .build();
///
/// // Restore existing actor state
/// let options = SpawnOptions::builder()
///     .exists(SpawnExistsOptions::Restore)
///     .build();
/// ```
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

/// A trait that defines the core functionality of an actor in the system.
///
/// Actors are isolated units of computation that communicate through message passing.
/// Each actor:
/// - Has unique identity (ActorId)
/// - Maintains private state
/// - Communicates only through messages
/// - Can be spawned, saved, and stopped
///
/// # Examples
///
/// ```rust
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Debug, Serialize, Deserialize)]
/// struct CounterActor {
///     count: usize,
/// }
///
/// #[derive(Debug, thiserror::Error)]
/// enum CounterError {
///     #[error("System error: {0}")]
///     System(#[from] SystemActorError),
/// }
///
/// impl ActorError for CounterError {}
///
/// impl Actor for CounterActor {
///     type Error = CounterError;
///
///     async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
///         // Actor's main loop
///         let mut stream = ctx.recv().await?;
///         while let Some(Ok(message)) = stream.next().await {
///             // Process messages...
///         }
///         Ok(())
///     }
/// }
///
/// // Spawn the actor
/// let engine = Engine::new().await?;
/// let id = ActorId::of::<CounterActor>("/counter");
/// let (ctx, actor) = CounterActor { count: 0 }
///     .spawn(engine, id, SpawnOptions::default())
///     .await?;
/// ```
pub trait Actor: Sized + Serialize + for<'de> Deserialize<'de> + Debug + Send + Sync {
    /// The error type returned by this actor's operations.
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
                        let ctx = ActorContext::new(engine.clone(), id.clone());
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
            let ctx = ActorContext::new(engine.clone(), id.clone());
            Ok((ctx, actor))
        }
    }

    /// Starts the actor's main loop.
    ///
    /// This function is the entry point for the actor's lifecycle.
    ///
    /// # Arguments
    ///
    /// * `&mut self` - A mutable reference to the actor instance.
    /// * `ctx` - A mutable reference to the actor's context, providing access to the actor's
    ///   state and communication methods.
    ///
    /// # Returns
    ///
    /// Returns a `Result<(), Self::Error>`:
    /// - `Ok(())` if the actor completes its work successfully.
    /// - `Err(Self::Error)` if an error occurs during the actor's execution.
    fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), Self::Error>>;

    /// Saves the current state of the actor in the system.
    ///
    /// This function updates the actor's state in the database.
    ///
    /// # Arguments
    ///
    /// * `ctx` - A reference to the `ActorContext` instance.
    ///
    /// # Returns
    ///
    /// A `Future` that resolves to a `Result` containing either:
    /// - `Ok(())`: The actor state was successfully saved.
    /// - `Err(Self::Error)`: An error if the saving process fails.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - Serialization of the actor state fails.
    /// - Updating the actor record in the database fails.
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

/// The context for an actor, providing access to the actor system.
///
/// The context allows an actor to:
/// - Send and receive messages
/// - Stream replies to messages
/// - Access the underlying database
/// - Manage actor lifecycle
///
/// Each actor instance has its own context that is created when the
/// actor is spawned.
#[derive(Debug)]
pub struct ActorContext<T: Actor> {
    /// The actor system engine
    engine: Engine,
    /// The actor's unique identifier
    id: ActorId,
    /// Channel for sending reply chunks during message processing
    tx: Option<mpsc::UnboundedSender<Value>>,
    /// Type marker for the actor
    _marker: std::marker::PhantomData<T>,
}

impl<T: Actor> ActorContext<T> {
    /// Create a new actor context
    fn new(engine: Engine, id: ActorId) -> Self {
        debug!("[{}] ctx-new", id.record_id());
        Self { engine, id, tx: None, _marker: std::marker::PhantomData }
    }

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

    /// Begins processing an incoming message and sets up reply streaming.
    ///
    /// This method:
    /// 1. Creates a channel for streaming replies
    /// 2. Spawns task to handle reply sending
    /// 3. Sets up message context for replies
    ///
    /// # Arguments
    ///
    /// * `frame` - The message frame being processed
    ///
    /// # Returns
    ///
    /// Returns a JoinHandle for the reply processing task
    async fn start_message_processing(&mut self, frame: FrameMessage) -> tokio::task::JoinHandle<()> {
        debug!("[{}] msg-process-start {} {}", self.id().record_id(), frame.name, frame.id);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let engine = self.engine.clone();
        let frame_clone = frame.clone();

        let handle = tokio::spawn(async move {
            let chunk_counter = AtomicU64::new(1);

            while let Some(value) = rx.recv().await {
                let chunk = chunk_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                debug!("[{}] msg-chunk {} {}-{}", frame_clone.rx, frame_clone.name, frame_clone.id.key(), chunk);

                let reply = FrameReply::new_chunk(
                    frame_clone.id.key().to_string(),
                    chunk,
                    frame_clone.name.clone(),
                    frame_clone.rx.clone(),
                    frame_clone.tx.clone(),
                    value,
                );

                let reply_id = reply.id.to_record_id();

                let reply_query = include_str!("../sql/reply.surql");
                let result = engine
                    .db()
                    .query(reply_query)
                    .bind(("reply_id", reply_id))
                    .bind(("reply", reply))
                    .bind(("msg_id", frame_clone.id.clone()))
                    .await;
                if let Err(e) = result {
                    error!(
                        "[{}] msg-chunk-error {} {}-{} {}",
                        frame_clone.rx,
                        frame_clone.name,
                        frame_clone.id.key(),
                        chunk,
                        e
                    );
                }
            }

            // Store the values we'll need for logging before using frame_clone
            let rx = frame_clone.rx.clone();
            let name = frame_clone.name.clone();
            let id_key = frame_clone.id.key().to_string();

            debug!("[{}] msg-final {} {}", rx, name, id_key);

            let reply = FrameReply::new_final(id_key.clone(), frame_clone.name, frame_clone.rx, frame_clone.tx);

            let reply_id = reply.id.to_record_id();

            let reply_query = include_str!("../sql/reply.surql");
            if let Err(e) = engine
                .db()
                .query(reply_query)
                .bind(("reply_id", reply_id))
                .bind(("reply", reply))
                .bind(("msg_id", frame_clone.id))
                .await
            {
                error!("[{}] msg-final-error {} {} {}", rx, name, id_key, e);
            }
        });

        self.tx = Some(tx);
        handle
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
    /// It's useful for fire-and-forget type operations where you don't need to process a reply.
    ///
    /// # Type Parameters
    ///
    /// * `M`: The message handler type, which must implement `Message<MT>`.
    /// * `MT`: The message type, which must implement `MessageType`.
    ///
    /// # Arguments
    ///
    /// * `message`: The message to be sent.
    /// * `to`: The `ActorId` of the recipient actor.
    ///
    /// # Returns
    ///
    /// A `Result` which is:
    /// - `Ok(())` if the message was successfully sent.
    /// - `Err(SystemActorError)` if there was an error in sending the message.
    ///
    /// # Errors
    ///
    /// This method will return an error if the message preparation or sending process fails.
    pub async fn do_send<M, MT>(&self, message: MT, to: &ActorId) -> Result<(), SystemActorError>
    where
        M: Message<MT>,
        MT: MessageType,
    {
        let _ = self.prepare_and_send_message::<MT>(&message, to).await?;
        Ok(())
    }

    /// Send a message and receive a stream of replies.
    ///
    /// # Type Parameters
    ///
    /// * `M` - Message handler type
    /// * `MT` - Message content type
    ///
    /// # Arguments
    ///
    /// * `message` - Message to send
    /// * `to` - Recipient actor's ID
    /// * `options` - Send configuration options
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing:
    /// - `Ok(ReplyStream<M::Response>)` - Stream of replies
    /// - `Err(SystemActorError)` - If send fails
    ///
    /// # Example
    ///
    /// ```rust
    /// async fn query_data(&self, ctx: &mut ActorContext<Self>) -> Result<(), Error> {
    ///     let mut replies = ctx.send::<DataActor, QueryMessage>(
    ///         QueryMessage::new("user.*"),
    ///         &data_actor_id,
    ///         SendOptions::default()
    ///     ).await?;
    ///
    ///     while let Some(reply) = replies.next().await {
    ///         match reply {
    ///             Ok(data) => println!("Received: {:?}", data),
    ///             Err(e) => break
    ///         }
    ///     }
    ///     Ok(())
    /// }
    /// ```
    pub async fn send<M, MT>(
        &self,
        message: MT,
        to: &ActorId,
        options: SendOptions,
    ) -> Result<ReplyStream<M::Response>, SystemActorError>
    where
        M: Message<MT>,
        MT: MessageType,
        M::Response: 'static,
    {
        let (_, reply_id, _) = self.prepare_and_send_message::<MT>(&message, to).await?;
        self.wait_for_replies::<M::Response>(&reply_id, options).await
    }

    /// Send a message to an actor without waiting for a reply.
    ///
    /// This method allows sending a message to an actor without expecting a response.
    /// It's useful for fire-and-forget type operations where you don't need to wait
    /// for or process a reply.
    ///
    /// # Type Parameters
    ///
    /// * `MT`: The type of the message being sent, which must implement `MessageType`.
    ///
    /// # Arguments
    ///
    /// * `message`: The message to be sent.
    /// * `to`: The `ActorId` of the recipient actor.
    ///
    /// # Returns
    ///
    /// A `Result` which is:
    /// - `Ok(())` if the message was successfully sent.
    /// - `Err(SystemActorError)` if there was an error in sending the message.
    ///
    /// # Errors
    ///
    /// This method will return an error if the message preparation or sending process fails.
    pub async fn do_send_as<MT>(&self, message: MT, to: &ActorId) -> Result<(), SystemActorError>
    where
        MT: MessageType,
    {
        let (_, _, _) = self.prepare_and_send_message(&message, to).await?;
        Ok(())
    }

    /// Send a message to an actor and wait for a reply.
    ///
    /// This method is similar to `send` but allows for sending a message to an actor
    /// and waiting for a reply without knowing if the actor can handle the message type.
    ///
    /// # Type Parameters
    ///
    /// * `MT`: The type of the message being sent.
    /// * `RT`: The expected type of the reply.
    ///
    /// # Arguments
    ///
    /// * `message`: The message to be sent.
    /// * `to`: The `ActorId` of the recipient actor.
    /// * `options`: The `SendOptions` for this message.
    ///
    /// # Returns
    ///
    /// A `Result` containing either:
    /// - `Ok(RT)`: The successfully received reply of type `RT`.
    /// - `Err(SystemActorError)`: An error if the sending or receiving process fails.
    pub async fn send_as<MT, RT>(
        &self,
        message: MT,
        to: ActorId,
        options: SendOptions,
    ) -> Result<ReplyStream<RT>, SystemActorError>
    where
        MT: MessageType,
        RT: MessageType + 'static,
    {
        let (_, reply_id, _) = self.prepare_and_send_message::<MT>(&message, &to).await?;
        self.wait_for_replies::<RT>(&reply_id, options).await
    }

    /// Waits for and streams all replies to a sent message.
    ///
    /// This method establishes a live query to receive replies as they arrive. The reply
    /// stream will continue until a final reply (with `chunk = None`) is received or an
    /// error occurs.
    ///
    /// # Type Parameters
    ///
    /// * `RT` - The expected type of the reply messages.
    ///
    /// # Arguments
    ///
    /// * `reply_id` - The ID of the reply to wait for
    /// * `options` - Options controlling timeout and other behaviors
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing either:
    /// - `Ok(ReplyStream<RT>)` - A stream of reply messages that can be consumed
    /// - `Err(SystemActorError)` - If setting up the reply stream fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// async fn handle_replies(ctx: &ActorContext<MyActor>) -> Result<(), SystemActorError> {
    ///     // Send message and get reply stream
    ///     let mut replies = ctx.send::<TargetActor, MyMessage>(
    ///         message,
    ///         &target_id,
    ///         SendOptions::default()
    ///     ).await?;
    ///
    ///     // Process replies as they arrive
    ///     while let Some(reply) = replies.next().await {
    ///         match reply {
    ///             Ok(response) => println!("Got response: {:?}", response),
    ///             Err(e) => {
    ///                 eprintln!("Error: {}", e);
    ///                 break;
    ///             }
    ///         }
    ///     }
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Error Handling
    ///
    /// This method will return an error if:
    /// - The live query setup fails
    /// - Database connection is lost
    /// - Query timeout is reached
    ///
    /// Individual stream items may contain errors if:
    /// - Reply deserialization fails
    /// - Reply contains an error message
    /// - Stream timeout is reached while waiting for next reply
    async fn wait_for_replies<RT: MessageType + 'static>(
        &self,
        reply_id: &RecordId,
        options: SendOptions,
    ) -> Result<ReplyStream<RT>, SystemActorError> {
        // Debug print for starting to wait for replies
        debug!("[{}] reply-wait {} {}", self.id().record_id(), std::any::type_name::<RT>(), reply_id.key());

        // Set up the live query for replies
        let query =
            format!("LIVE SELECT id.{{id, chunk}}, * FROM {} WHERE id.id = '{}'", DB_TABLE_REPLY, reply_id.key());
        debug!("[{}] reply-live {}", self.id().record_id(), query);

        let mut res = self.engine().db().query(&query).await?;
        let notification_stream = res.stream::<Notification<FrameReply>>(0)?;
        let self_id = self.id().clone();

        // Transform the notification stream into a reply stream
        let stream = notification_stream
            // Only process Create actions
            .filter(|n| future::ready(matches!(n, Ok(n) if n.action == Action::Create)))
            .map(|n| -> Result<FrameReply, SystemActorError> {
                let n = n?;
                Ok(n.data)
            })
            // Take messages until we get final message (chunk = None)
            .take_while(|reply| {
                future::ready(match reply {
                    Ok(reply) => reply.id.chunk.is_some(), // Continue while we have chunks
                    Err(_) => false,                       // Stop on error
                })
            })
            // Process each reply
            .map(move |reply| -> Result<RT, SystemActorError> {
                let reply = reply?;

                // Debug print for each received reply
                debug!(
                    "[{}] reply-recv {} {} chunk={:?}",
                    self_id.record_id(),
                    std::any::type_name::<RT>(),
                    reply.id.id,
                    reply.id.chunk
                );

                // Check for error responses
                if !reply.err.is_null() {
                    return Err(SystemActorError::MessageReply(reply.err.to_string().into()));
                }

                // Deserialize the message content
                let response: RT = serde_json::from_value(reply.msg)?;
                Ok(response)
            });

        // Apply timeout to the entire stream
        let stream = Box::pin(stream);
        let stream = stream.then(move |r| async move {
            match timeout(options.timeout, future::ready(r)).await {
                Ok(msg) => msg,
                Err(_) => Err(SystemActorError::MessageTimeout(std::any::type_name::<RT>().into(), options.timeout)),
            }
        });

        Ok(Box::pin(stream))
    }

    /// Send a reply as part of processing a message.
    ///
    /// This method can be called multiple times during message processing to
    /// send a stream of replies. The replies will be sent in order and each
    /// will be assigned a chunk number.
    ///
    /// # Type Parameters
    ///
    /// * `R` - The type of the reply content
    ///
    /// # Arguments
    ///
    /// * `response` - The reply content to send
    ///
    /// # Returns
    ///
    /// Returns a `Result` which is:
    /// - `Ok(())` if the reply was sent successfully
    /// - `Err(SystemActorError)` if sending failed
    ///
    /// # Errors
    ///
    /// This method will return an error if:
    /// - No message is currently being processed
    /// - The reply channel has been closed
    /// - Serialization of the reply content fails
    pub async fn reply<R: MessageType>(&self, response: R) -> Result<(), SystemActorError> {
        if let Some(tx) = &self.tx {
            let value = serde_json::to_value(&response)?;
            tx.send(value).map_err(|_| SystemActorError::MessageReply("Reply channel closed".into()))?;
            Ok(())
        } else {
            Err(SystemActorError::MessageReply("No active message processing".into()))
        }
    }

    // Called when handle() returns to clean up
    async fn finish_message_processing(&mut self) {
        // Drop channel to trigger final reply
        self.tx = None;
    }

    /// Receive messages for this actor
    ///
    /// This method sets up a stream of messages for the actor, combining any unreplied messages
    /// with a live query for new incoming messages.
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing:
    /// - `Ok(MessageStream)`: A pinned box containing a stream of `Result<FrameMessage, SystemActorError>`.
    /// - `Err(SystemActorError)`: If there's an error setting up the message stream.
    ///
    /// # Errors
    ///
    /// This method can return an error if:
    /// - There's an issue retrieving unreplied messages.
    /// - The live query setup fails.
    /// - There's an error in the database query.
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
                let mut pong_stream = ctx.send::<PongActor, Ping>(Ping, &pong_id, SendOptions::default()).await?;

                // Collect response from stream
                if let Some(pong) = pong_stream.next().await {
                    let pong = pong?;
                    info!("{} Pong {}", ctx.id(), pong.times);
                    if pong.times == 0 {
                        break;
                    }
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

        async fn handle(&mut self, _ctx: &mut ActorContext<Self>, _message: &Ping) -> Result<(), Self::Error> {
            if self.times == 0 {
                Err(PongActorError::PongLimitReached)
            } else {
                self.times -= 1;
                Ok(())
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
