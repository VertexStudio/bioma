use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BehaviorTick;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum BehaviorStatus {
    Success,
    Failure,
}

pub trait Behavior: Sized {}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionBehavior<B: Behavior> {
    pub node: B,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecoratorBehavior<B: Behavior> {
    pub node: B,
    pub child: ActorId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompositeBehavior<B: Behavior> {
    pub node: B,
    pub children: Vec<ActorId>,
}
