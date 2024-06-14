use crate::{
    behavior::{BehaviorStatus, BehaviorTelemetry, BehaviorTelemetryInfo},
    error::BehaviorError,
};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::{any::Any, fmt::Debug};
use tokio::sync::{broadcast, Mutex};

type BlackboardChannelType = dyn Any + Send;

#[typetag::serde(tag = "type")]
pub trait BlackboardChannelBus: Send {
    fn subscribe(&mut self) -> Box<BlackboardChannelType>;
    fn publisher(&mut self) -> Box<BlackboardChannelType>;

    
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StringChannel {
    capacity: usize,
    #[serde(skip)]
    runtime: Option<BlackboardChannelRuntime<String>>,
}

impl StringChannel {
    pub fn new(capacity: usize) -> Box<Self> {
        Box::new(Self {
            capacity,
            runtime: None,
        })
    }
}

#[derive(Debug)]
struct BlackboardChannelRuntime<T> {
    sender: broadcast::Sender<T>,
    _receiver: broadcast::Receiver<T>,
}

#[macro_export]
macro_rules! impl_blackboard_channel_bus {
    () => {
        fn subscribe(&mut self) -> Box<BlackboardChannelType> {
            if self.runtime.is_none() {
                let (sender, _receiver) = broadcast::channel(self.capacity);
                let runtime = BlackboardChannelRuntime { sender, _receiver };
                self.runtime = Some(runtime);
            }
            let runtime = self.runtime.as_ref().unwrap();
            let sender = runtime.sender.clone();
            Box::new(sender.subscribe())
        }
    
        fn publisher(&mut self) -> Box<BlackboardChannelType> {
            if self.runtime.is_none() {
                let (sender, _receiver) = broadcast::channel(self.capacity);
                let runtime = BlackboardChannelRuntime { sender, _receiver };
                self.runtime = Some(runtime);
            }
            let runtime = self.runtime.as_ref().unwrap();
            let sender = runtime.sender.clone();
            Box::new(sender)
        }
    };
}

#[typetag::serde(name = "bioma::core::StringChannel")]
impl BlackboardChannelBus for StringChannel {
    impl_blackboard_channel_bus!();
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct BlackboardChannel(Cow<'static, str>);

impl BlackboardChannel {
    pub fn new<S: Into<Cow<'static, str>>>(name: S) -> Self {
        Self(name.into())
    }
}

impl std::fmt::Display for BlackboardChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Serialize, Deserialize)]
pub struct Blackboard {
    pubsubs: HashMap<BlackboardChannel, Box<dyn BlackboardChannelBus>>,
}

impl Debug for Blackboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Blackboard")
            .field("pubsubs", &self.pubsubs.keys())
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct BlackboardHandle {
    pub blackboard: Arc<Mutex<Blackboard>>,
}

impl BlackboardHandle {
    pub fn new() -> Self {
        let blackboard = Blackboard::new();
        Self {
            blackboard: Arc::new(Mutex::new(blackboard)),
        }
    }

    pub fn with(blackboard: Blackboard) -> Self {
        Self {
            blackboard: Arc::new(Mutex::new(blackboard)),
        }
    }

    pub async fn add_channel(
        &self,
        name: &BlackboardChannel,
        channel: Box<dyn BlackboardChannelBus>,
    ) {
        let mut bb = self.blackboard.lock().await;
        bb.add_channel(name, channel);
    }

    pub async fn del_channel(
        &self,
        name: &BlackboardChannel,
    ) -> Option<Box<dyn BlackboardChannelBus>> {
        let mut bb = self.blackboard.lock().await;
        bb.del_channel(name)
    }

    pub async fn subscribe<T: 'static>(
        &self,
        channel: &BlackboardChannel,
        telemetry_info: Option<BehaviorTelemetryInfo>,
        status: Option<Result<BehaviorStatus, BehaviorError>>,
    ) -> Result<BlackboardSubscriber<T>, BehaviorError> {
        let mut bb = self.blackboard.lock().await;
        bb.subscribe(channel, telemetry_info, status).await
    }

    pub async fn publisher<T: 'static>(
        &self,
        channel: &BlackboardChannel,
        telemetry_info: Option<BehaviorTelemetryInfo>,
        status: Option<Result<BehaviorStatus, BehaviorError>>,
    ) -> Result<BlackboardPublisher<T>, BehaviorError> {
        let mut bb = self.blackboard.lock().await;
        bb.publisher(channel, telemetry_info, status).await
    }
}

/// Publisher for sending messages to a blackboard channel.
pub struct BlackboardPublisher<T> {
    channel: BlackboardChannel,
    sender: broadcast::Sender<T>,
    telemetry_info: Option<BehaviorTelemetryInfo>,
}

impl Debug for BlackboardPublisher<String> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlackboardPublisher")
            .field("channel", &self.channel)
            .finish()
    }
}

impl<T> BlackboardPublisher<T>
where
    T: Serialize,
{
    pub async fn send(
        &self,
        msg: T,
        status: Option<Result<BehaviorStatus, BehaviorError>>,
    ) -> Result<usize, broadcast::error::SendError<T>> {
        if let Some(telemetry_info) = &self.telemetry_info {
            let log = serde_json::to_string(&msg).ok();
            if let Some(tx) = telemetry_info.telemetry_tx.as_ref() {
                let _ = tx
                    .send(BehaviorTelemetry {
                        tree_id: telemetry_info.tree_id.clone(),
                        name: telemetry_info.node_name.clone(),
                        node_id: telemetry_info.node_id.clone(),
                        status: status.clone(),
                        log: log.map(|l| format!("SEND:{} {}", self.channel, l).into()),
                    })
                    .await;
            }
        }
        self.sender.send(msg)
    }
}

/// Subscriber for receiving messages from a blackboard channel.
pub struct BlackboardSubscriber<T> {
    channel: BlackboardChannel,
    receiver: broadcast::Receiver<T>,
    telemetry_info: Option<BehaviorTelemetryInfo>,
}

impl Debug for BlackboardSubscriber<String> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlackboardSubscriber")
            .field("channel", &self.channel)
            .finish()
    }
}

impl<T> BlackboardSubscriber<T>
where
    T: Clone + Serialize,
{
    pub async fn recv(
        &mut self,
        status: Option<Result<BehaviorStatus, BehaviorError>>,
    ) -> Result<T, broadcast::error::RecvError> {
        let msg = self.receiver.recv().await?;
        if let Some(telemetry_info) = &self.telemetry_info {
            let log = serde_json::to_string(&msg).ok();
            if let Some(tx) = telemetry_info.telemetry_tx.as_ref() {
                let _ = tx
                    .send(BehaviorTelemetry {
                        tree_id: telemetry_info.tree_id.clone(),
                        name: telemetry_info.node_name.clone(),
                        node_id: telemetry_info.node_id.clone(),
                        status: status.clone(),
                        log: log.map(|l| format!("RECV:{} {}", self.channel, l).into()),
                    })
                    .await;
            }
        }
        Ok(msg)
    }
}

impl Blackboard {
    fn new() -> Self {
        Self {
            pubsubs: HashMap::new(),
        }
    }

    fn add_channel(&mut self, name: &BlackboardChannel, channel: Box<dyn BlackboardChannelBus>) {
        self.pubsubs.insert(name.clone(), channel);
    }

    fn del_channel(&mut self, name: &BlackboardChannel) -> Option<Box<dyn BlackboardChannelBus>> {
        self.pubsubs.remove(&name)
    }

    async fn subscribe<T: 'static>(
        &mut self,
        channel: &BlackboardChannel,
        telemetry_info: Option<BehaviorTelemetryInfo>,
        status: Option<Result<BehaviorStatus, BehaviorError>>,
    ) -> Result<BlackboardSubscriber<T>, BehaviorError> {
        if let Some(pubsub) = self.pubsubs.get_mut(channel) {
            let sub = pubsub.subscribe();
            let sub = sub.downcast::<broadcast::Receiver<T>>();
            if let Ok(sub) = sub {
                if let Some(telemetry_info) = &telemetry_info {
                    let log = Some(format!("SUB:{}", channel).into());
                    if let Some(tx) = telemetry_info.telemetry_tx.as_ref() {
                        let _ = tx
                            .send(BehaviorTelemetry {
                                tree_id: telemetry_info.tree_id.clone(),
                                name: telemetry_info.node_name.clone(),
                                node_id: telemetry_info.node_id.clone(),
                                status: status.clone(),
                                log,
                            })
                            .await;
                    }
                }

                let bbs = BlackboardSubscriber {
                    channel: channel.clone(),
                    receiver: *sub,
                    telemetry_info,
                };
                return Ok(bbs);
            }
        }
        Err(BehaviorError::InvalidBlackboardChannel(
            format!("{}:[{}]", channel, std::any::type_name::<T>()).into(),
        ))
    }

    async fn publisher<T: 'static>(
        &mut self,
        channel: &BlackboardChannel,
        telemetry_info: Option<BehaviorTelemetryInfo>,
        status: Option<Result<BehaviorStatus, BehaviorError>>,
    ) -> Result<BlackboardPublisher<T>, BehaviorError> {
        if let Some(pubsub) = self.pubsubs.get_mut(channel) {
            let publ = pubsub.publisher();
            let publ = publ.downcast::<broadcast::Sender<T>>();
            if let Ok(publ) = publ {
                if let Some(telemetry_info) = &telemetry_info {
                    let log = Some(format!("PUB:{}", channel).into());
                    if let Some(tx) = telemetry_info.telemetry_tx.as_ref() {
                        let _ = tx
                            .send(BehaviorTelemetry {
                                tree_id: telemetry_info.tree_id.clone(),
                                name: telemetry_info.node_name.clone(),
                                node_id: telemetry_info.node_id.clone(),
                                status: status.clone(),
                                log,
                            })
                            .await;
                    }
                }

                let bbp = BlackboardPublisher {
                    channel: channel.clone(),
                    sender: *publ,
                    telemetry_info,
                };
                return Ok(bbp);
            }
        }
        Err(BehaviorError::InvalidBlackboardChannel(
            format!("{}:[{}]", channel, std::any::type_name::<T>()).into(),
        ))
    }
}
