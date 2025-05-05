use crate::schema::{CallToolResult, TextContent};
use crate::server::RequestContext;
use crate::tools::ToolDef;
use anyhow::{anyhow, Error};
use lazy_static::lazy_static;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

lazy_static! {
    static ref MEMORY_STORE: Mutex<HashMap<String, Value>> = Mutex::new(HashMap::new());
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MemoryAction {
    Store,
    Retrieve,
    List,
    Delete,
    Clear,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct MemoryArgs {
    #[schemars(required = true)]
    #[schemars(
        description = "The action to perform: 'store' to save a value, 'retrieve' to get a value, 'list' to see all keys, 'delete' to remove a key, or 'clear' to remove all keys"
    )]
    action: MemoryAction,

    #[schemars(description = "The key to store/retrieve/delete the memory under (not required for list/clear)")]
    key: Option<String>,

    #[schemars(description = "The JSON object to store (only required for store action)")]
    value: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Memory;

impl ToolDef for Memory {
    const NAME: &'static str = "memory";
    const DESCRIPTION: &'static str = "Store and retrieve JSON memories using string keys";
    type Args = MemoryArgs;

    async fn call(&self, args: Self::Args, _request_context: RequestContext) -> Result<CallToolResult, Error> {
        let store_result = MEMORY_STORE.lock();
        let mut store = match store_result {
            Ok(store) => store,
            Err(e) => return Ok(Self::error(e.to_string())),
        };

        let result = match args.action {
            MemoryAction::Store => {
                let key = match args.key {
                    Some(k) => k,
                    None => return Ok(Self::error("Key is required for store action")),
                };
                let value = match args.value {
                    Some(v) => v,
                    None => return Ok(Self::error("Value is required for store action")),
                };
                store.insert(key.clone(), value);
                format!("Successfully stored memory with key: {}", key)
            }
            MemoryAction::Retrieve => {
                let key = match args.key {
                    Some(k) => k,
                    None => return Ok(Self::error("Key is required for retrieve action")),
                };
                match store.get(&key) {
                    Some(value) => {
                        serde_json::to_string_pretty(value).map_err(|e| anyhow!("Failed to serialize result: {}", e))?
                    }
                    None => format!("No memory found for key: {}", key),
                }
            }
            MemoryAction::List => {
                let keys: Vec<&String> = store.keys().collect();
                match serde_json::to_string_pretty(&keys) {
                    Ok(json_str) => json_str,
                    Err(e) => return Ok(Self::error(format!("Failed to serialize keys: {}", e))),
                }
            }
            MemoryAction::Delete => {
                let key = match args.key {
                    Some(k) => k,
                    None => return Ok(Self::error("Key is required for delete action")),
                };
                match store.remove(&key) {
                    Some(_) => format!("Successfully deleted memory with key: {}", key),
                    None => format!("No memory found to delete for key: {}", key),
                }
            }
            MemoryAction::Clear => {
                store.clear();
                "Successfully cleared all memories".to_string()
            }
        };

        Ok(Self::success(result))
    }
}

impl Memory {
    fn error(error_message: impl Into<String>) -> CallToolResult {
        CallToolResult {
            content: vec![serde_json::to_value(TextContent {
                type_: "text".to_string(),
                text: error_message.into(),
                annotations: None,
            })
            .unwrap_or_default()],
            is_error: Some(true),
            meta: None,
        }
    }

    fn success(message: impl Into<String>) -> CallToolResult {
        CallToolResult {
            content: vec![serde_json::to_value(TextContent {
                type_: "text".to_string(),
                text: message.into(),
                annotations: None,
            })
            .unwrap_or_default()],
            is_error: Some(false),
            meta: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolCallHandler;
    use serde_json::json;

    async fn clear_memory() {
        let tool = Memory;
        let clear_props = MemoryArgs { action: MemoryAction::Clear, key: None, value: None };
        tool.call(clear_props, RequestContext::default()).await.unwrap();
    }

    #[test]
    fn test_auto_generated_schema() {
        let tool = Memory.def();
        let schema_json = serde_json::to_string_pretty(&tool).unwrap();
        println!("Tool Schema:\n{}", schema_json);
    }

    #[tokio::test]
    async fn test_memory_operations() {
        clear_memory().await;

        let tool = Memory;

        let store_props = MemoryArgs {
            action: MemoryAction::Store,
            key: Some("test_key".to_string()),
            value: Some(json!({"test": "value"})),
        };
        let result = tool.call(store_props, RequestContext::default()).await.unwrap();
        assert!(result.content[0]["text"].as_str().unwrap().contains("Successfully stored"));

        let retrieve_props =
            MemoryArgs { action: MemoryAction::Retrieve, key: Some("test_key".to_string()), value: None };
        let result = tool.call(retrieve_props, RequestContext::default()).await.unwrap();
        assert!(result.content[0]["text"].as_str().unwrap().contains("test"));

        let list_props = MemoryArgs { action: MemoryAction::List, key: None, value: None };
        let result = tool.call(list_props, RequestContext::default()).await.unwrap();
        assert!(result.content[0]["text"].as_str().unwrap().contains("test_key"));

        let delete_props = MemoryArgs { action: MemoryAction::Delete, key: Some("test_key".to_string()), value: None };
        let result = tool.call(delete_props, RequestContext::default()).await.unwrap();
        assert!(result.content[0]["text"].as_str().unwrap().contains("Successfully deleted"));

        let store_props = MemoryArgs {
            action: MemoryAction::Store,
            key: Some("test_key2".to_string()),
            value: Some(json!({"test": "value"})),
        };
        tool.call(store_props, RequestContext::default()).await.unwrap();

        let clear_props = MemoryArgs { action: MemoryAction::Clear, key: None, value: None };
        let result = tool.call(clear_props, RequestContext::default()).await.unwrap();
        assert!(result.content[0]["text"].as_str().unwrap().contains("Successfully cleared"));

        let list_props = MemoryArgs { action: MemoryAction::List, key: None, value: None };
        let result = tool.call(list_props, RequestContext::default()).await.unwrap();
        assert_eq!(result.content[0]["text"].as_str().unwrap(), "[]");
    }

    #[tokio::test]
    async fn test_memory_value_types() {
        clear_memory().await;
        let tool = Memory;

        let test_cases = vec![
            ("object_key", json!({"test": "value"})),
            ("array_key", json!([1, 2, 3])),
            ("string_key", json!("test string")),
            ("number_key", json!(42)),
            ("boolean_key", json!(true)),
            ("null_key", json!(null)),
        ];

        for (key, value) in test_cases {
            let store_props =
                MemoryArgs { action: MemoryAction::Store, key: Some(key.to_string()), value: Some(value.clone()) };
            let result = tool.call(store_props, RequestContext::default()).await.unwrap();
            assert!(result.content[0]["text"].as_str().unwrap().contains("Successfully stored"));

            let retrieve_props = MemoryArgs { action: MemoryAction::Retrieve, key: Some(key.to_string()), value: None };
            let result = tool.call(retrieve_props, RequestContext::default()).await.unwrap();
            let retrieved_value: Value = serde_json::from_str(result.content[0]["text"].as_str().unwrap()).unwrap();
            assert_eq!(retrieved_value, value);
        }
    }
}
