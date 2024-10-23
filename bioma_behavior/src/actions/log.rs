use crate::prelude::*;
use bioma_actor::prelude::*;
use bon::Builder;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

/// Logs a message at the specified level.
///
/// The `Log` action logs a message when ticked and always returns success.
#[derive(Builder, Debug, Serialize, Deserialize)]
pub struct Log {
    pub level: LogLevel,
    pub text: String,
    #[serde(skip)]
    #[builder(skip)]
    pub node: behavior::Action,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
}

impl Behavior for Log {
    fn node(&self) -> behavior::Node {
        behavior::Node::Action(&self.node)
    }
}

pub struct LogFactory;

impl ActorFactory for LogFactory {
    fn spawn(
        &self,
        engine: Engine,
        config: serde_json::Value,
        id: ActorId,
        options: SpawnOptions,
    ) -> Result<ActorHandle, SystemActorError> {
        let engine = engine.clone();
        let node: tree::ActionNode = serde_json::from_value(config.clone()).unwrap();
        let config: Log = serde_json::from_value(node.data.config.clone())?;
        Ok(tokio::spawn(async move {
            let (mut ctx, mut actor) = Actor::spawn(engine, id, config, options).await?;
            debug!("LogFactory::spawn: start {}", ctx.id());
            actor.start(&mut ctx).await?;
            debug!("LogFactory::spawn: end {}", ctx.id());
            Ok(())
        }))
    }
}

impl Message<BehaviorTick> for Log {
    type Response = BehaviorStatus;

    async fn handle(
        &mut self,
        _ctx: &mut ActorContext<Self>,
        _msg: &BehaviorTick,
    ) -> Result<BehaviorStatus, Self::Error> {
        match self.level {
            LogLevel::Error => error!("{}", self.text),
            LogLevel::Warn => warn!("{}", self.text),
            LogLevel::Info => info!("{}", self.text),
        }
        Ok(BehaviorStatus::Success)
    }
}

impl Actor for Log {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(BehaviorTick) = frame.is::<BehaviorTick>() {
                self.reply(ctx, &BehaviorTick, &frame).await?;
                break;
            }
        }
        Ok(())
    }
}
