use crate::error::BehaviorError;
use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(Debug, Serialize, Deserialize)]
struct Node {
    tag: Cow<'static, str>,
    uid: Cow<'static, str>,
    children: Vec<ActorId>,
    #[serde(skip)]
    start_handle: Option<ActorHandle>,
}

struct ActorHandle(Box<dyn Future<Output = Result<(), BehaviorError>>>);

impl std::fmt::Debug for ActorHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActorHandle").finish()
    }
}
