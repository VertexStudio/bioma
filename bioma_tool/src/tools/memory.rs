use crate::schema::{CallToolResult, TextContent, Tool, ToolInputSchema};
use crate::tools::{ToolDef, ToolError};
use lazy_static::lazy_static;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

pub const MEMORY_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "description": "The action to perform: 'store' to save a value, 'retrieve' to get a value, 'list' to see all keys, 'delete' to remove a key, or 'clear' to remove all keys",
            "type": "string",
            "enum": ["store", "retrieve", "list", "delete", "clear"]
        },
        "key": {
            "description": "The key to store/retrieve/delete the memory under (not required for list/clear)",
            "type": "string"
        },
        "value": {
            "description": "The JSON value to store (only required for store action)",
            "type": ["object", "null"]
        }
    },
    "required": ["action"]
}"#;

// Global memory store
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
pub struct MemoryProperties {
    #[schemars(required = true)]
    #[schemars(
        description = "The action to perform: 'store' to save a value, 'retrieve' to get a value, 'list' to see all keys, 'delete' to remove a key, or 'clear' to remove all keys"
    )]
    #[schemars(with = "String")]
    action: MemoryAction,

    #[schemars(
        description = "The key to store/retrieve/delete the memory under (not required for list/clear)"
    )]
    key: Option<String>,

    #[schemars(description = "The JSON value to store (only required for store action)")]
    #[schemars(with = "Value")]
    value: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Memory;

impl ToolDef for Memory {
    const NAME: &'static str = "memory";
    const DESCRIPTION: &'static str = "Store and retrieve JSON memories using string keys";
    type Properties = MemoryProperties;

    fn def() -> Tool {
        let input_schema = serde_json::from_str::<ToolInputSchema>(MEMORY_SCHEMA).unwrap();
        Tool {
            name: Self::NAME.to_string(),
            description: Some(Self::DESCRIPTION.to_string()),
            input_schema,
        }
    }

    async fn call(&self, properties: Self::Properties) -> Result<CallToolResult, ToolError> {
        let store_result = MEMORY_STORE.lock();
        let mut store = match store_result {
            Ok(store) => store,
            Err(e) => return Ok(Self::error(e.to_string())),
        };

        let result = match properties.action {
            MemoryAction::Store => {
                let key = match properties.key {
                    Some(k) => k,
                    None => return Ok(Self::error("Key is required for store action")),
                };
                let value = match properties.value {
                    Some(v) => v,
                    None => return Ok(Self::error("Value is required for store action")),
                };
                store.insert(key.clone(), value);
                format!("Successfully stored memory with key: {}", key)
            }
            MemoryAction::Retrieve => {
                let key = match properties.key {
                    Some(k) => k,
                    None => return Ok(Self::error("Key is required for retrieve action")),
                };
                match store.get(&key) {
                    Some(value) => serde_json::to_string_pretty(value)
                        .map_err(|e| ToolError::ResultSerialize(e))?,
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
                let key = match properties.key {
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
        let clear_props = MemoryProperties {
            action: MemoryAction::Clear,
            key: None,
            value: None,
        };
        tool.call(clear_props).await.unwrap();
    }

    #[tokio::test]
    async fn test_memory_operations() {
        clear_memory().await;

        let tool = Memory;

        // Test storing
        let store_props = MemoryProperties {
            action: MemoryAction::Store,
            key: Some("test_key".to_string()),
            value: Some(json!({"test": "value"})),
        };
        let result = tool.call(store_props).await.unwrap();
        assert!(result.content[0]["text"]
            .as_str()
            .unwrap()
            .contains("Successfully stored"));

        // Test retrieving
        let retrieve_props = MemoryProperties {
            action: MemoryAction::Retrieve,
            key: Some("test_key".to_string()),
            value: None,
        };
        let result = tool.call(retrieve_props).await.unwrap();
        assert!(result.content[0]["text"].as_str().unwrap().contains("test"));

        // Test listing
        let list_props = MemoryProperties {
            action: MemoryAction::List,
            key: None,
            value: None,
        };
        let result = tool.call(list_props).await.unwrap();
        assert!(result.content[0]["text"]
            .as_str()
            .unwrap()
            .contains("test_key"));

        // Test deleting
        let delete_props = MemoryProperties {
            action: MemoryAction::Delete,
            key: Some("test_key".to_string()),
            value: None,
        };
        let result = tool.call(delete_props).await.unwrap();
        assert!(result.content[0]["text"]
            .as_str()
            .unwrap()
            .contains("Successfully deleted"));

        // Test clearing
        let store_props = MemoryProperties {
            action: MemoryAction::Store,
            key: Some("test_key2".to_string()),
            value: Some(json!({"test": "value"})),
        };
        tool.call(store_props).await.unwrap();

        let clear_props = MemoryProperties {
            action: MemoryAction::Clear,
            key: None,
            value: None,
        };
        let result = tool.call(clear_props).await.unwrap();
        assert!(result.content[0]["text"]
            .as_str()
            .unwrap()
            .contains("Successfully cleared"));

        // Verify memory is empty after clear
        let list_props = MemoryProperties {
            action: MemoryAction::List,
            key: None,
            value: None,
        };
        let result = tool.call(list_props).await.unwrap();
        assert_eq!(result.content[0]["text"].as_str().unwrap(), "[]");
    }

    #[tokio::test]
    async fn test_memory_input_schema() {
        clear_memory().await;

        let tool = Memory.def();
        let input_schema = tool.input_schema;

        assert_eq!(input_schema.type_, "object");

        // Safely get properties
        let properties = input_schema.properties.expect("Should have properties");

        // Check action property
        let action_prop = properties
            .get("action")
            .expect("Should have action property");
        assert_eq!(
            action_prop.get("type").and_then(|v| v.as_str()),
            Some("string")
        );

        // Check enum values exist for action
        let enum_values = action_prop
            .get("enum")
            .and_then(|v| v.as_array())
            .expect("Should have enum values for action");

        // Verify all action types are present
        assert!(enum_values.contains(&json!("store")));
        assert!(enum_values.contains(&json!("retrieve")));
        assert!(enum_values.contains(&json!("list")));
        assert!(enum_values.contains(&json!("delete")));
        assert!(enum_values.contains(&json!("clear")));

        // Check key and value properties exist
        assert!(properties.contains_key("key"), "Should have key property");
        assert!(
            properties.contains_key("value"),
            "Should have value property"
        );

        // Check required fields
        let required = input_schema.required.expect("Should have required fields");
        assert!(
            required.contains(&"action".to_string()),
            "Action should be required"
        );
        assert_eq!(required.len(), 1, "Only action should be required");
    }

    #[test]
    fn test_auto_generated_schema() {
        let tool = Memory.def();
        println!("Tool: {:?}", tool);
    }
}
