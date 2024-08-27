use bioma_actor::dbg_export_db;
use bioma_actor::prelude::*;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::future::Future;
use std::time::Duration;
use test_log::test;
use tokio::time::timeout;
use tracing::info;

const ACTOR_HOST: &str = "/host";
const ACTOR_GUEST: &str = "/guest";

#[derive(Clone, Serialize, Deserialize)]
struct Hello {
    subject: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct HelloResponse {
    greeting: String,
}

struct Host {
    id: ActorId,
}

impl Host {
    async fn new(uid: impl Into<Cow<'static, str>>) -> Result<Self, ActorError> {
        Ok(Self { id: ActorId::spawn_of::<Self>(uid).await? })
    }
}

impl ActorModel for Host {
    fn id(&self) -> &ActorId {
        &self.id
    }

    fn start(&mut self) -> impl Future<Output = Result<(), ActorError>> {
        async move {
            let mut stream = self.recv().await?;
            while let Some(Ok(frame)) = stream.next().await {
                if let Some(message) = self.is::<Self, Hello>(&frame) {
                    let response = self.handle(&message).await?;
                    self.reply::<Self, Hello>(&frame, response).await?;
                }
            }
            Ok(())
        }
    }
}

impl Message<Hello> for Host {
    type Response = HelloResponse;

    fn handle(&mut self, message: &Hello) -> impl Future<Output = Result<Self::Response, ActorError>> {
        async move { Ok(HelloResponse { greeting: format!("Hello, {}!", message.subject) }) }
    }
}

struct Guest {
    id: ActorId,
}

impl Guest {
    async fn new(uid: impl Into<Cow<'static, str>>) -> Result<Self, ActorError> {
        Ok(Self { id: ActorId::spawn_of::<Self>(uid).await? })
    }
}

impl ActorModel for Guest {
    fn id(&self) -> &ActorId {
        &self.id
    }

    fn start(&mut self) -> impl Future<Output = Result<(), ActorError>> {
        async move {
            let host = ActorId::of::<Host>(ACTOR_HOST);

            let message = Hello { subject: "guest".to_string() };
            let response: HelloResponse = self.send::<Host, Hello>(message, &host).await?;
            info!("Response: {}", response.greeting);

            let message = Hello { subject: "world".to_string() };
            let response: HelloResponse = self.send::<Host, Hello>(message, &host).await?;
            info!("Response: {}", response.greeting);

            Ok(())
        }
    }
}

#[test(tokio::test)]
async fn test_message_hello_world() -> Result<(), ActorError> {
    EE.test().await.unwrap();

    // Spawn the host actor
    let mut host = Host::new(ACTOR_HOST).await.unwrap();
    let host_handle = tokio::spawn(async move {
        host.start().await.unwrap();
    });

    // Spawn the guest actor
    let mut guest = Guest::new(ACTOR_GUEST).await.unwrap();
    guest.start().await.unwrap();

    // Terminate the host actor
    host_handle.abort();

    // Ensure the host actor has terminated
    match timeout(Duration::from_secs(1), host_handle).await {
        Ok(_) => (),
        Err(_) => panic!("Host actor failed to terminate"),
    }

    dbg_export_db!();

    Ok(())
}
