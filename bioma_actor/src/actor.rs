// use crate::dbg_export_db;
use crate::engine::DB;
use crate::error::ActorError;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use surrealdb::{
    sql::{Id, Thing},
    Action, Notification,
};
use tracing::{debug, error};

const DB_TABLE_ACTOR: &str = "actor";
const DB_TABLE_MESSAGE: &str = "message";
const DB_TABLE_REPLY: &str = "reply";

#[derive(Debug, Deserialize)]
struct PostRecord {
    #[allow(dead_code)]
    id: Thing,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Post {
    id: Thing,
    pub name: Cow<'static, str>,
    pub tx: Thing,
    pub rx: Thing,
    pub msg: Value,
}

/// A unique identifier for a distributed actor
#[derive(Clone, Debug, Serialize, Deserialize, Eq, Hash, PartialEq)]
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
        let _actor: Vec<Actor> = DB.create(DB_TABLE_ACTOR).content(actor).await?;
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

    /// Post a message to the actor and wait for a reply
    pub async fn send(
        &self,
        name: impl Into<Cow<'static, str>>,
        to: &ActorId,
        message: &Value,
    ) -> Result<Value, ActorError> {
        let name = name.into();

        let message_id = Id::ulid();

        let request_id = Thing::from((DB_TABLE_MESSAGE, message_id.clone()));
        let reply_id = Thing::from((DB_TABLE_REPLY, message_id.clone()));

        let request = Post {
            id: request_id.clone(),
            name: name.clone(),
            tx: self.id.clone(),
            rx: to.id.clone(),
            msg: message.clone(),
        };

        debug!(
            "msg-send {} {} {} -> {} {}",
            name,
            request.id,
            self,
            to,
            serde_json::to_string(&message)?
        );

        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let post_ids: Result<Vec<PostRecord>, _> =
                DB.create(DB_TABLE_MESSAGE).content(request).await;
            if let Ok(post_ids) = post_ids {
                let id = post_ids[0].id.clone();
                if request_id != id {
                    error!(
                        "msg-send {}",
                        request_id
                    );
                }
            }
        });

        // Live wait for reply
        let mut stream = DB.select(reply_id).live().await?;

        // dbg_export_db!();

        let Some(notification) = stream.next().await else {
            return Err(ActorError::LiveStream(format!("Empty: {}", name).into()));
        };
        let notification: Notification<Post> = notification?;
        match notification.action {
            Action::Create => {
                let data = &notification.data;
                debug!(
                    "msg-done {} {} {} -> {} {}",
                    data.name,
                    data.id,
                    data.tx,
                    data.rx,
                    serde_json::to_string(&data.msg)?
                );
                let reply = serde_json::to_value(&data.msg)?;
                Ok(reply)
            }
            _ => Err(ActorError::LiveStream(
                format!("!Action::Create: {}", name).into(),
            )),
        }
    }

    /// Reply to a message
    pub async fn reply(&self, request: &Post, message: Value) -> Result<(), ActorError> {
        // Use the request id as the reply id
        let reply_id = Thing::from((DB_TABLE_REPLY, request.id.id.clone()));

        let reply = Post {
            id: reply_id.clone(),
            name: request.name.clone(),
            tx: request.rx.clone(),
            rx: request.tx.clone(),
            msg: message,
        };

        debug!(
            "msg-rply {} {} {} -> {} {}",
            reply.name,
            reply_id,
            reply.tx,
            reply.rx,
            serde_json::to_string(&reply.msg)?
        );

        let _entry: Vec<Post> = DB.create(DB_TABLE_REPLY).content(reply).await?;

        Ok(())
    }

    /// Receive messages
    pub async fn recv(
        &self,
    ) -> Result<impl Stream<Item = Result<Notification<Post>, surrealdb::Error>>, ActorError> {
        let query = format!(
            "LIVE SELECT * FROM {} WHERE rx = {}",
            DB_TABLE_MESSAGE, self.id
        );
        debug!("msg-live {} {}", self, &query);
        let mut res = DB.query(&query).await?;
        let live_query = res.stream::<Notification<Post>>(0)?;
        let self_id = self.id.clone();
        let live_query = live_query
            // .filter(|item| {
            //     // Filter out non-create actions
            //     let should_filter = item
            //         .as_ref()
            //         .ok()
            //         .map(|notification| notification.action == Action::Create)
            //         .unwrap_or(false);
            //     async move { should_filter }
            // })
            .inspect(move |item| match item {
                Ok(notification) => {
                    let post = &notification.data;
                    let tag = notification_to_tag(&notification);
                    debug!(
                        "msg-recv {} {} {} -> {}{}{}",
                        post.name,
                        post.id,
                        post.tx,
                        post.rx,
                        tag,
                        serde_json::to_string(&post.msg).unwrap_or("".into())
                    );
                }
                Err(error) => debug!("msg-recv {} {:?}", self_id, error),
            });
        Ok(live_query)
    }
}

fn notification_to_tag<T>(notification: &Notification<T>) -> &str {
    match notification.action {
        Action::Create => " ",
        Action::Update => " UPDATE ",
        Action::Delete => " DELETE ",
        _ => "!",
    }
}

impl std::fmt::Display for ActorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.id)
    }
}
