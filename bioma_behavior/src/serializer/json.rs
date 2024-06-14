use crate::blackboard::*;
use crate::prelude::*;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc;

pub async fn from_json<Config>(
    value: &serde_json::Value,
    telemetry_tx: Option<mpsc::Sender<BehaviorTelemetry>>,
) -> Result<BehaviorTreeHandle, BehaviorError>
where
    Config: BehaviorTreeConfig + for<'de> Deserialize<'de> + 'static,
{
    // Ensure value is an object
    let Some(value) = value.as_object().clone() else {
        return Err(BehaviorError::ParseError("Invalid JSON object".into()));
    };

    // Extract tree id
    let Some(tree_id) = value.get("id") else {
        return Err(BehaviorError::ParseError("Missing tree id".into()));
    };
    let Ok(tree_id) = serde_json::from_value::<String>(tree_id.clone()) else {
        return Err(BehaviorError::ParseError("Invalid tree id".into()));
    };
    let tree_id = BehaviorTreeId::new(tree_id);

    // Extract custom configuration
    let Some(config) = value.get("config") else {
        return Err(BehaviorError::ParseError("Missing config".into()));
    };
    let Ok(config) = serde_json::from_value::<Config>(config.clone()) else {
        return Err(BehaviorError::ParseError("Invalid config".into()));
    };
    let config = Box::new(config);

    // Extract optional blakcboard
    let blackboard: Result<Blackboard, _> = serde_json::from_value(value["blackboard"].clone());
    let blackboard = blackboard
        .ok()
        .map(|bb: Blackboard| BlackboardHandle::with(bb));

    // Extract root node id
    let Some(root) = value.get("root") else {
        return Err(BehaviorError::ParseError("Missing root".into()));
    };
    let Ok(root) = serde_json::from_value::<BehaviorId>(root.clone()) else {
        return Err(BehaviorError::ParseError("Invalid root".into()));
    };

    // Extract all the nodes
    let mut nodes: IndexMap<BehaviorId, Option<Box<dyn BehaviorNode>>> = IndexMap::new();

    let Some(json_nodes) = value.get("nodes") else {
        return Err(BehaviorError::ParseError("Missing nodes".into()));
    };
    let Some(json_nodes) = json_nodes.as_array() else {
        return Err(BehaviorError::ParseError(
            "Invalid nodes, expected array".into(),
        ));
    };

    for json_node in json_nodes {
        let Some(node_id) = json_node.get("id") else {
            return Err(BehaviorError::ParseError("Missing node id".into()));
        };
        let Ok(node_id) = serde_json::from_value::<BehaviorId>(node_id.clone()) else {
            return Err(BehaviorError::ParseError("Invalid node id".into()));
        };

        let Some(node) = json_node.get("node") else {
            return Err(BehaviorError::ParseError("Missing node".into()));
        };
        let node = serde_json::to_string(node).unwrap();
        let Ok(node) = serde_json::from_str(&node) else {
            return Err(BehaviorError::ParseError("Invalid node".into()));
        };

        nodes.insert(node_id, Some(node));
    }

    let mut bt = BehaviorTreeHandle::new(BehaviorTree::new(
        &tree_id,
        &root,
        config,
        telemetry_tx,
        blackboard,
    ));
    for (node_id, node) in &mut nodes {
        let node = node.take().unwrap();
        bt.add_node(&node_id, node).await;
    }

    Ok(bt)
}

pub async fn to_json<Config>(
    bt: &BehaviorTreeHandle,
) -> Result<serde_json::Value, serde_json::Error>
where
    Config: Serialize + Sized,
{
    let bt = bt.lock().await;

    // Collect all nodes
    let mut nodes = vec![];
    for (key, node) in &bt.nodes {
        let node = node.node();
        let node = node.lock().await;
        let node = json!({
            "id": key,
            "node": *node,
        });
        nodes.push(node);
    }

    // Serialize the tree
    let config = serde_json::to_value(bt.config.as_ref())?;
    let mut tree_json = json!({
        "id": bt.id().clone(),
        "config": config,
        "root": bt.root,
        "nodes": nodes,
    });

    if let Some(blackboard) = &bt.blackboard {
        let blackboard = blackboard.blackboard.lock().await;
        let blackboard: &Blackboard = &blackboard;
        let blackboard = serde_json::to_value(blackboard)?;
        let tree_json = tree_json.as_object_mut().unwrap();
        tree_json.insert("blackboard".into(), blackboard);
    }

    Ok(tree_json)
}
