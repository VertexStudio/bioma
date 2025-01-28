use crate::engine::{Engine, Record};
use futures::{future, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use surrealdb::sql;
use surrealdb::RecordIdKey;
use tokio::sync::mpsc;
// use std::any::type_name;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, SystemTime};
use std::{borrow::Cow, sync::atomic::AtomicU64};
use surrealdb::{sql::Id, value::RecordId, Action, Notification};
use tracing::{debug, error, trace};

// Constants for database table names
const DB_TABLE_ACTOR: &str = "actor";
const DB_TABLE_MESSAGE: &str = "message";
const DB_TABLE_REPLY: &str = "reply";
const DB_TABLE_HEALTH: &str = "health";

/// Implement this trait to define custom actor error types
pub trait ActorError: std::error::Error + Debug + Send + Sync + From<SystemActorError> {}

/// Enumerates the types of errors that can occur in Actor framework
#[derive(thiserror::Error, Debug)]
pub enum SystemActorError {
    /// Underlying IO error from the system.
    ///
    /// This typically occurs during file operations or network communication.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Error from the SurrealDB database operations.
    ///
    /// Occurs during database queries, transactions, or connection issues with SurrealDB.
    #[error("Engine error: {0}")]
    EngineError(#[from] surrealdb::Error),

    /// Error when attempting to deserialize a message into an incorrect type.
    ///
    /// This happens when a received message's type name doesn't match the expected type
    /// during message handling or reply processing.
    #[error("MessageTypeMismatch: {0}")]
    MessageTypeMismatch(&'static str),

    /// Error in the live query subscription stream.
    ///
    /// Occurs when there are issues with the real-time message or reply streams,
    /// typically during actor communication.
    #[error("LiveStream error: {0}")]
    LiveStream(Cow<'static, str>),

    /// JSON serialization/deserialization error for messages or actor state.
    ///
    /// This occurs when converting between Rust types and JSON representation
    /// for message content or actor persistence.
    #[error("JsonSerde error: {0}")]
    JsonSerde(#[from] serde_json::Error),

    /// Mismatch between expected and actual record IDs.
    ///
    /// This error occurs when database record IDs don't match during actor
    /// operations, typically indicating a consistency issue.
    #[error("Id mismatch: {0:?} {1:?}")]
    IdMismatch(RecordId, RecordId),

    /// Mismatch between actor type tags.
    ///
    /// Occurs when an actor's type tag doesn't match its expected type,
    /// usually during actor spawning or message routing.
    #[error("Actor tag mismatch: {0} {1}")]
    ActorTagMismatch(Cow<'static, str>, Cow<'static, str>),

    /// Attempt to spawn an actor with an ID that already exists.
    ///
    /// This error occurs when trying to create a new actor instance with
    /// SpawnOptions::Error and the actor ID is already in use.
    #[error("Actor already exists: {0}")]
    ActorAlreadyExists(ActorId),

    /// Error in the message reply process.
    ///
    /// Occurs during reply handling, such as when a reply channel is closed
    /// or when trying to reply without an active message context.
    #[error("Message reply error: {0}")]
    MessageReply(Cow<'static, str>),

    /// Timeout while waiting for a message reply.
    ///
    /// This error occurs when a reply to a message isn't received within
    /// the specified timeout duration in SendOptions.
    #[error("Message timeout {0} {1:?}")]
    MessageTimeout(Cow<'static, str>, std::time::Duration),

    /// Task execution timeout.
    ///
    /// Occurs when an actor task exceeds its allocated execution time.
    #[error("Tasked timeout after {0:?}")]
    TaskTimeout(std::time::Duration),

    /// Error from the object store operations.
    ///
    /// This occurs during interactions with the object storage system,
    /// such as saving or loading large objects.
    #[error("Object store error: {0}")]
    ObjectStore(#[from] object_store::Error),

    /// Invalid path in object store operations.
    ///
    /// Occurs when attempting to use an invalid path format
    /// in object store operations.
    #[error("Object store error: {0}")]
    PathError(#[from] object_store::path::Error),

    /// URL parsing error.
    ///
    /// Occurs when trying to parse invalid URLs, typically
    /// for external resource access or configuration.
    #[error("Url error: {0}")]
    UrlError(#[from] url::ParseError),

    /// Error from a Tokio task join handle.
    ///
    /// This occurs when a spawned task fails to complete or
    /// is cancelled, typically during actor lifecycle operations.
    #[error("Join handle error: {0}")]
    JoinHandle(#[from] tokio::task::JoinError),

    /// Attempt to register an actor tag that is already registered.
    ///
    /// This error occurs during actor system initialization when
    /// trying to register duplicate actor types.
    #[error("Actor tag already registered: {0}")]
    ActorTagAlreadyRegistered(Cow<'static, str>),

    /// Referenced actor tag is not registered in the system.
    ///
    /// This occurs when trying to interact with an actor type
    /// that hasn't been registered with the actor system.
    #[error("Actor tag not found: {0}")]
    ActorTagNotFound(Cow<'static, str>),

    /// Error when attempting to communicate with an unhealthy actor.
    ///
    /// This occurs when trying to send a message to an actor that hasn't
    /// updated its health status within the configured interval.
    #[error("Actor is unhealthy: {0}")]
    UnhealthyActor(ActorId),
}

impl ActorError for SystemActorError {}

/// The message frame that is sent between actors
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrameMessage {
    /// Message id created with a ULID
    id: RecordId,
    /// Message name (usually message type name)
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

    /// Creates a new error reply frame for a chunk in a streaming response
    pub fn new_chunk_error(
        id: String,
        chunk_num: u64,
        name: Cow<'static, str>,
        tx: surrealdb::RecordId,
        rx: surrealdb::RecordId,
        err: Value,
    ) -> Self {
        Self { id: ReplyId::new_chunk(id, chunk_num), name, tx, rx, msg: Value::Null, err }
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
{
    /// The type of response this message handler produces.
    type Response: Clone + Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static;

    /// Handles a message of type `MT` for this actor.
    ///
    /// This function must be implemented for any actor that wants to handle messages of type `MT`.
    /// It defines how the actor processes the incoming message and what response it generates.
    ///
    /// # Arguments
    ///
    /// * `self` - A mutable reference to the actor instance.
    /// * `ctx` - A mutable reference to the actor's context, providing access to the actor's state and environment. You can use `ctx.reply()` to send responses.   
    /// * `message` - A reference to the message of type `MT` to be handled.
    ///
    /// # Returns
    ///
    /// A `Result` which is:
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
    /// * `ctx` - A mutable reference to the actor context
    /// * `message` - A reference to the message to be handled
    /// * `frame` - A reference to the original message frame
    ///
    /// # Returns
    ///
    /// A `Result` which is:
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

            // If error, send error to client
            if let Err(e) = &result {
                ctx.error(e).await?;
            }

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
            let _ = ctx.prepare_and_send_message::<MT>(&message, to, None).await?;
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
            let (_, reply_id, _) = ctx.prepare_and_send_message::<MT>(&message, to, Some(options.clone())).await?;
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
#[derive(bon::Builder, Clone)]
pub struct SendOptions {
    /// The maximum duration to wait for a reply before timing out.
    #[builder(default = default_timeout())]
    pub timeout: std::time::Duration,
    /// Whether to check actor health before sending messages
    #[builder(default = default_check_health())]
    pub check_health: bool,
}

fn default_timeout() -> std::time::Duration {
    std::time::Duration::from_secs(30)
}

fn default_check_health() -> bool {
    false
}

impl Default for SendOptions {
    fn default() -> Self {
        Self::builder().build()
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
    /// A new `ActorId` instance with the generated id and the type name of the actor.
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

    /// Creates a health record ID for this actor
    ///
    /// This method generates a RecordId for the health record associated with this actor.
    /// The health record ID uses the same name as the actor but in the health table.
    ///
    /// # Returns
    ///
    /// A `RecordId` for the health record
    pub fn health_id(&self) -> RecordId {
        RecordId::from_table_key(DB_TABLE_HEALTH, self.name.as_ref())
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
    #[builder(default = default_spawn_exists())]
    exists: SpawnExistsOptions,
    /// Health configuration for the actor
    /// Some = enabled with config, None = disabled
    health_config: Option<HealthConfig>,
}

fn default_spawn_exists() -> SpawnExistsOptions {
    SpawnExistsOptions::Error
}

impl Default for SpawnOptions {
    fn default() -> Self {
        Self::builder().build()
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
                engine.db().lock().await.select(&id.record_id()).await.map_err(SystemActorError::from)?;

            if let Some(actor_record) = actor_record {
                // Actor exists, apply options
                match options.exists {
                    SpawnExistsOptions::Reset => {
                        // Reset the actor by deleting its record
                        let _: Option<ActorRecord> =
                            engine.db().lock().await.delete(&id.record_id()).await.map_err(SystemActorError::from)?;
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
                        let mut ctx = ActorContext::new(engine.clone(), id.clone());
                        ctx.init_health(options.health_config.clone()).await?;
                        return Ok((ctx, actor));
                    }
                }
            }

            // At this point, we either need to create a new actor or reset an existing one

            // Serialize actor properties
            let actor_state = serde_json::to_value(&actor).map_err(SystemActorError::from)?;

            // Create or update actor record in the database
            let content = ActorRecord { id: id.record_id(), tag: id.tag.clone(), state: actor_state };
            let _record: Option<Record> = engine
                .db()
                .lock()
                .await
                .create(DB_TABLE_ACTOR)
                .content(content)
                .await
                .map_err(SystemActorError::from)?;

            // Create the context
            let mut ctx = ActorContext::new(engine.clone(), id.clone());

            // Initialize health monitoring with the provided config
            ctx.init_health(options.health_config.clone()).await?;

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
    /// A `Result<(), Self::Error>`:
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

            let _record: Option<Record> = ctx
                .engine()
                .db()
                .lock()
                .await
                .update(&record_id)
                .content(content)
                .await
                .map_err(SystemActorError::from)?;

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

/// Configuration for actor health monitoring
///
/// When Some, health monitoring is enabled with the specified configuration.
/// When None, health monitoring is disabled.
///
/// Health monitoring works by:
/// 1. Creating a health record when the actor is spawned (if enabled)
/// 2. Periodically updating a timestamp in the health record
/// 3. Checking the timestamp before sending messages to ensure the actor is healthy
///
/// # Example
///
/// ```rust
/// // Enable health monitoring with 60 second interval
/// let config = Some(HealthConfig {
///     update_interval: Duration::from_secs(60)
/// });
///
/// let options = SpawnOptions::builder()
///     .health_config(config)
///     .build();
///
/// let (ctx, actor) = MyActor::spawn(engine, id, actor, options).await?;
/// ```
#[derive(bon::Builder, Clone, Debug, Serialize, Deserialize)]
pub struct HealthConfig {
    /// Interval at which the actor updates its last_seen timestamp
    #[builder(default = sql::Duration::from_secs(60))]
    pub update_interval: sql::Duration,
}

/// Record for storing health status
#[derive(Clone, Debug, Serialize, Deserialize)]
struct HealthRecord {
    id: RecordId,
    last_seen: sql::Datetime,
    enabled: bool,
    update_interval: sql::Duration,
}

impl HealthRecord {
    fn new(id: RecordId, config: Option<&HealthConfig>) -> Self {
        match config {
            Some(config) => {
                Self { id, last_seen: sql::Datetime::default(), enabled: true, update_interval: config.update_interval }
            }
            None => Self {
                id,
                last_seen: sql::Datetime::default(),
                enabled: false,
                update_interval: sql::Duration::from_secs(60), // default value not used
            },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct UpdateHealth {
    last_seen: sql::Datetime,
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
    tx: Option<mpsc::UnboundedSender<Result<Value, Value>>>,
    /// Handle to health update task
    health_task: Option<tokio::task::JoinHandle<()>>,
    /// Type marker for the actor
    _marker: std::marker::PhantomData<T>,
}

impl<T: Actor> Drop for ActorContext<T> {
    fn drop(&mut self) {
        if let Some(handle) = self.health_task.take() {
            handle.abort();
        }
    }
}

impl<T: Actor> ActorContext<T> {
    /// Create a new actor context
    fn new(engine: Engine, id: ActorId) -> Self {
        debug!("[{}] ctx-new", id.record_id());
        Self { engine, id, tx: None, health_task: None, _marker: std::marker::PhantomData }
    }

    async fn unreplied_messages(&self) -> Result<Vec<FrameMessage>, SystemActorError> {
        let query = include_str!("../sql/unreplied_messages.surql");
        let mut res = self.engine().db().lock().await.query(query).bind(("rx", self.id.record_id())).await?;
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
            self.engine().db().lock().await.select(&self.id.record_id()).await.map_err(SystemActorError::from);
        if let Ok(Some(_)) = record {
            true
        } else {
            false
        }
    }

    /// Initialize health monitoring and start periodic health updates for this actor
    ///
    /// This function:
    /// 1. Creates initial health record if health monitoring is enabled
    /// 2. Establishes relation between actor and health record
    /// 3. Starts periodic health update task if enabled
    ///
    /// # Arguments
    ///
    /// * `config` - Optional health monitoring configuration
    ///
    /// # Returns
    ///
    /// * `Ok(())` if initialization and startup succeed
    /// * `Err(SystemActorError)` if any step fails
    pub async fn init_health(&mut self, config: Option<HealthConfig>) -> Result<(), SystemActorError> {
        debug!(
            "[{}] health-init enabled={} interval={:?}",
            self.id().name(),
            config.is_some(),
            config.as_ref().map(|c| c.update_interval)
        );

        let health_id = self.id().health_id();
        let health_record = HealthRecord::new(health_id.clone(), config.as_ref());

        // Explicitly specify Record type for upsert operation
        let _: Option<HealthRecord> = self
            .engine()
            .db()
            .lock()
            .await
            .upsert(&health_id)
            .content(health_record.clone())
            .await
            .map_err(SystemActorError::from)?;

        // Start health update task if enabled
        if let Some(config) = config {
            let engine = self.engine().clone();
            let update_interval = config.update_interval;
            let health_id = health_id.clone();
            let actor_name = self.id().name().to_string();

            let handle = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(update_interval.into()).await;

                    let update = UpdateHealth { last_seen: sql::Datetime::default() };

                    if let Err(e) =
                        engine.db().lock().await.update::<Option<HealthRecord>>(&health_id).merge(update).await
                    {
                        error!("[{}] health-update-error: {}", actor_name, e);
                        break;
                    }
                }
            });

            self.health_task = Some(handle);
        }

        Ok(())
    }

    /// Check if an actor is healthy based on its health record
    ///
    /// An actor is considered healthy if either:
    /// - Health monitoring is disabled for the actor
    /// - The actor has updated its health status within its configured update interval
    ///
    /// # Arguments
    ///
    /// * `actor_id` - ID of the actor to check
    ///
    /// # Returns
    ///
    /// * `Ok(true)` if the actor is healthy
    /// * `Ok(false)` if the actor is unhealthy or not found
    /// * `Err(SystemActorError)` if checking health status fails
    pub async fn check_actor_health(&self, actor_id: &ActorId) -> Result<bool, SystemActorError> {
        let health_id = actor_id.health_id();

        let health: Option<HealthRecord> =
            self.engine().db().lock().await.select(&health_id).await.map_err(SystemActorError::from)?;

        if let Some(health) = health {
            if !health.enabled {
                debug!("[{}] health-check {} disabled=true", self.id().name(), actor_id.name());
                return Ok(true);
            }

            let elapsed =
                SystemTime::now().duration_since(SystemTime::from(health.last_seen.0)).unwrap_or(Duration::MAX);

            let update_interval: std::time::Duration = health.update_interval.into();
            // Cap grace period at 1 second
            let grace_period = std::cmp::min(update_interval / 10, Duration::from_secs(1));
            let is_healthy = elapsed <= update_interval + grace_period;

            debug!(
                "[{}] health-check {} elapsed={:?} interval={:?} grace={:?} healthy={}",
                self.id().name(),
                actor_id.name(),
                elapsed,
                update_interval,
                grace_period,
                is_healthy
            );

            Ok(is_healthy)
        } else {
            debug!("[{}] health-check {} record-not-found", self.id().name(), actor_id.name());
            Ok(false)
        }
    }

    /// Kill the actor
    pub async fn kill(&self) -> Result<(), SystemActorError> {
        let _: Option<ActorRecord> =
            self.engine().db().lock().await.delete(&self.id.record_id()).await.map_err(SystemActorError::from)?;
        Ok(())
    }

    /// Receive messages for this actor
    ///
    /// This method sets up a stream of messages for the actor, combining any unreplied messages
    /// with a live query for new incoming messages.
    ///
    /// # Returns
    ///
    /// A `Result` containing:
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
        let mut res = self.engine().db().lock().await.query(&query).await?;
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
    /// A JoinHandle for the reply processing task
    async fn start_message_processing(&mut self, frame: FrameMessage) -> tokio::task::JoinHandle<()> {
        // Set up message processing and logging
        debug!("[{}] msg-process-start {} {}", self.id().record_id(), frame.name, frame.id);

        // Create an unbounded channel for streaming replies
        let (tx, mut rx) = mpsc::unbounded_channel::<Result<Value, Value>>();

        // Clone database and frame for use in spawned task
        let db = self.engine.db().clone();
        let frame_clone = frame.clone();

        // Load SQL query template for reply insertion
        let reply_query = include_str!("../sql/reply.surql");

        // Spawn async task to handle reply processing
        let handle = tokio::spawn(async move {
            // Counter for tracking reply chunks in stream
            let chunk_counter = AtomicU64::new(1);

            while let Some(result) = rx.recv().await {
                let chunk = chunk_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                debug!("[{}] msg-chunk {} {}-{}", frame_clone.rx, frame_clone.name, frame_clone.id.key(), chunk);

                // Create reply frame based on Ok/Err
                let reply = match result {
                    Ok(msg) => FrameReply::new_chunk(
                        frame_clone.id.key().to_string(),
                        chunk,
                        frame_clone.name.clone(),
                        frame_clone.rx.clone(),
                        frame_clone.tx.clone(),
                        msg,
                    ),
                    Err(err) => FrameReply::new_chunk_error(
                        frame_clone.id.key().to_string(),
                        chunk,
                        frame_clone.name.clone(),
                        frame_clone.rx.clone(),
                        frame_clone.tx.clone(),
                        err,
                    ),
                };

                // Get database ID for reply
                let reply_id = reply.id.to_record_id();

                // Insert reply chunk into database
                // This query likely creates a new reply record and links it to original message
                let result = db
                    .lock()
                    .await
                    .query(reply_query)
                    .bind(("reply_id", reply_id))
                    .bind(("reply", reply))
                    .bind(("msg_id", frame_clone.id.clone()))
                    .await;

                // Log any errors during reply insertion
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

            // After channel closes (all replies sent), send final reply
            // Store values needed for logging
            let rx = frame_clone.rx.clone();
            let name = frame_clone.name.clone();
            let id_key = frame_clone.id.key().to_string();

            debug!("[{}] msg-final {} {}", rx, name, id_key);

            // Create final reply frame (chunk = None indicates end of stream)
            let reply = FrameReply::new_final(id_key.clone(), frame_clone.name, frame_clone.rx, frame_clone.tx);
            let reply_id = reply.id.to_record_id();

            // Insert final reply into database
            // Uses same query template but marks this as final reply
            if let Err(e) = db
                .lock()
                .await
                .query(reply_query)
                .bind(("reply_id", reply_id))
                .bind(("reply", reply))
                .bind(("msg_id", frame_clone.id))
                .await
            {
                error!("[{}] msg-final-error {} {} {}", rx, name, id_key, e);
            }
        });

        // Store sender in context for later reply sending
        self.tx = Some(tx);
        handle
    }

    // Called when handle() returns to clean up
    async fn finish_message_processing(&mut self) {
        // Drop channel to trigger final reply
        self.tx = None;
    }

    /// Internal method to prepare and send a message
    async fn prepare_and_send_message<MT>(
        &self,
        message: &MT,
        to: &ActorId,
        options: Option<SendOptions>,
    ) -> Result<(RecordId, RecordId, FrameMessage), SystemActorError>
    where
        MT: MessageType,
    {
        // Check health if enabled
        if options.is_some() && options.unwrap().check_health {
            let is_healthy = self.check_actor_health(to).await?;
            if !is_healthy {
                return Err(SystemActorError::UnhealthyActor(to.clone()));
            }
        }

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
                db.lock().await.create(DB_TABLE_MESSAGE).content(task_request).await;
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
        let _ = self.prepare_and_send_message::<MT>(&message, to, None).await?;
        Ok(())
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
        let (_, _, _) = self.prepare_and_send_message(&message, to, None).await?;
        Ok(())
    }

    /// Send a message and receive a stream of replies.
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
    /// * `options`: The `SendOptions` for this message.
    ///
    /// # Returns
    ///
    /// A `Result` containing:
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
    {
        let (_, reply_id, _) = self.prepare_and_send_message::<MT>(&message, to, Some(options.clone())).await?;
        self.wait_for_replies::<M::Response>(&reply_id, options).await
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
        let (_, reply_id, _) = self.prepare_and_send_message::<MT>(&message, &to, Some(options.clone())).await?;
        self.wait_for_replies::<RT>(&reply_id, options).await
    }

    /// Sends a message and collects all replies into a Vec.
    ///
    /// This is a convenience method that handles the boilerplate of collecting
    /// all items from a reply stream after sending a message.
    ///
    /// # Type Parameters
    ///
    /// * `M`: The message handler type
    /// * `MT`: The message type being sent
    ///
    /// # Arguments
    ///
    /// * `message`: The message to send
    /// * `to`: The recipient actor ID  
    /// * `options`: Send options controlling timeout and other behaviors
    ///
    /// # Returns
    ///
    /// A `Result` containing either:
    /// - `Ok(Vec<M::Response>)`: All collected replies
    /// - `Err(SystemActorError)`: If sending or collecting fails
    pub async fn send_and_collect<M, MT>(
        &self,
        message: MT,
        to: &ActorId,
        options: SendOptions,
    ) -> Result<Vec<M::Response>, SystemActorError>
    where
        M: Message<MT>,
        MT: MessageType,
    {
        let mut stream = self.send::<M, MT>(message, to, options).await?;
        let mut results = Vec::new();

        while let Some(result) = stream.next().await {
            results.push(result?);
        }

        Ok(results)
    }

    /// Sends a message and waits for exactly one reply.
    ///
    /// This is a convenience method for cases where only a single reply is expected.
    /// It will return an error if more than one reply is received.
    ///
    /// # Type Parameters
    ///
    /// * `M`: The message handler type
    /// * `MT`: The message type being sent
    ///
    /// # Arguments
    ///
    /// * `message`: The message to send
    /// * `to`: The recipient actor ID
    /// * `options`: Send options controlling timeout and other behaviors
    ///
    /// # Returns
    ///
    /// A `Result` containing either:
    /// - `Ok(M::Response)`: The single reply
    /// - `Err(SystemActorError)`: If sending fails, no reply is received, or multiple replies are received
    pub async fn send_and_wait_reply<M, MT>(
        &self,
        message: MT,
        to: &ActorId,
        options: SendOptions,
    ) -> Result<M::Response, SystemActorError>
    where
        M: Message<MT>,
        MT: MessageType,
    {
        let mut stream = self.send::<M, MT>(message, to, options).await?;

        if let Some(first) = stream.next().await {
            let result = first?;

            // Ensure there are no additional replies
            if stream.next().await.is_some() {
                return Err(SystemActorError::MessageReply("Expected single reply but received multiple".into()));
            }

            Ok(result)
        } else {
            Err(SystemActorError::MessageReply("No reply received".into()))
        }
    }

    /// Sends a message to an actor and waits for exactly one reply without knowing the handler type.
    ///
    /// This is a convenience method that combines `send_as` and waiting for a single reply.
    /// It will return an error if more than one reply is received.
    ///
    /// # Type Parameters
    ///
    /// * `MT`: The type of the message being sent
    /// * `RT`: The expected type of the reply
    ///
    /// # Arguments
    ///
    /// * `message`: The message to send
    /// * `to`: The recipient actor ID
    /// * `options`: Send options controlling timeout and other behaviors
    ///
    /// # Returns
    ///
    /// A `Result` containing either:
    /// - `Ok(RT)`: The single reply
    /// - `Err(SystemActorError)`: If sending fails, no reply is received, or multiple replies are received
    pub async fn send_as_and_wait_reply<MT, RT>(
        &self,
        message: MT,
        to: ActorId,
        options: SendOptions,
    ) -> Result<RT, SystemActorError>
    where
        MT: MessageType,
        RT: MessageType + 'static,
    {
        let mut stream = self.send_as::<MT, RT>(message, to, options).await?;

        if let Some(first) = stream.next().await {
            let result = first?;

            // Ensure there are no additional replies
            if stream.next().await.is_some() {
                return Err(SystemActorError::MessageReply("Expected single reply but received multiple".into()));
            }

            Ok(result)
        } else {
            Err(SystemActorError::MessageReply("No reply received".into()))
        }
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
    /// A `Result` which is:
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
            tx.send(Ok(value)).map_err(|_| SystemActorError::MessageReply("Reply channel closed".into()))?;
            Ok(())
        } else {
            Err(SystemActorError::MessageReply("No active message processing".into()))
        }
    }

    /// Sends an error response during message processing.
    ///
    /// The error will be propagated through the reply stream to the sender.
    ///
    /// # Arguments
    ///
    /// * `error` - The error to send, must implement ActorError
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the error was sent successfully
    /// * `Err(SystemActorError)` if sending the error failed
    pub async fn error(&self, error: &impl ActorError) -> Result<(), SystemActorError> {
        if let Some(tx) = &self.tx {
            let value = serde_json::to_value(&error.to_string())?;
            tx.send(Err(value)).map_err(|_| SystemActorError::MessageReply("Reply channel closed".into()))?;
            Ok(())
        } else {
            Err(SystemActorError::MessageReply("No active message processing".into()))
        }
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
    /// A `Result` containing either:
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

        let mut res = self.engine().db().lock().await.query(&query).await?;
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
                debug!(
                    "[{}] reply-recv {} {} chunk={:?}",
                    self_id.record_id(),
                    std::any::type_name::<RT>(),
                    reply.id.id,
                    reply.id.chunk
                );

                if !reply.err.is_null() {
                    return Err(SystemActorError::MessageReply(reply.err.to_string().into()));
                }

                let response: RT = serde_json::from_value(reply.msg)?;
                Ok(response)
            });

        // Timeout stream that maps all items through a timeout
        let timeout_duration = options.timeout;
        let stream =
            futures::stream::unfold((Box::pin(stream), timeout_duration), |(mut stream, timeout)| async move {
                match tokio::time::timeout(timeout, stream.next()).await {
                    Ok(Some(item)) => Some((item, (stream, timeout))),
                    Ok(None) => None,
                    Err(_) => {
                        error!(
                            "Stream item timeout after {:?} for message type {}",
                            timeout,
                            std::any::type_name::<RT>()
                        );
                        Some((
                            Err(SystemActorError::MessageTimeout(std::any::type_name::<RT>().into(), timeout)),
                            (stream, timeout),
                        ))
                    }
                }
            });

        Ok(Box::pin(stream))
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

                let pong =
                    ctx.send_and_wait_reply::<PongActor, Ping>(Ping, &pong_id, SendOptions::builder().build()).await?;
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

        async fn handle(&mut self, ctx: &mut ActorContext<Self>, _message: &Ping) -> Result<(), Self::Error> {
            if self.times == 0 {
                Err(PongActorError::PongLimitReached)
            } else {
                self.times -= 1;
                ctx.reply(Pong { times: self.times }).await?;
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
