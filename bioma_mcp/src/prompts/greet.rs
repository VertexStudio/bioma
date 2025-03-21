use crate::prompts::{PromptDef, PromptError};
use crate::schema::{GetPromptResult, PromptMessage, Role, TextContent};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct GreetArgs {
    #[schemars(description = "Name of the person to greet")]
    name: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Greet;

impl PromptDef for Greet {
    const NAME: &'static str = "greet";
    const DESCRIPTION: &'static str = "A friendly greeting prompt";
    type Args = GreetArgs;

    fn def() -> crate::schema::Prompt {
        crate::schema::Prompt {
            name: Self::NAME.to_string(),
            description: Some(Self::DESCRIPTION.to_string()),
            arguments: Some(vec![crate::schema::PromptArgument {
                name: "name".to_string(),
                description: Some("Name of the person to greet".to_string()),
                required: Some(true),
            }]),
        }
    }

    async fn get(&self, args: Self::Args) -> Result<GetPromptResult, PromptError> {
        Ok(GetPromptResult {
            messages: vec![
                PromptMessage {
                    role: Role::Assistant,
                    content: serde_json::to_value(TextContent {
                        text: "What's your name?, After you tell me, I will let you know that I am Bioma, your AI assistant.".to_string(),
                        type_: "text".to_string(),
                        annotations: None,
                    })
                    .map_err(PromptError::ResultSerialize)?,
                },
                PromptMessage {
                    role: Role::User,
                    content: serde_json::to_value(TextContent {
                        text: args.name.clone(),
                        type_: "text".to_string(),
                        annotations: None,
                    })
                    .map_err(PromptError::ResultSerialize)?,
                },
            ],
            description: Some("A friendly greeting prompt".to_string()),
            meta: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_greet() {
        let greet = Greet;
        let props = GreetArgs { name: "Alice".to_string() };

        let result = greet.get(props).await.unwrap();
        let text_content: TextContent = serde_json::from_value(result.messages[0].content.clone()).unwrap();
        assert_eq!(text_content.text, "Hello, Alice! Welcome to Bioma!");
    }

    #[test]
    fn test_greet_schema() {
        let prompt = Greet::def();
        assert_eq!(prompt.name, "greet");
        assert_eq!(prompt.description.unwrap(), "A friendly greeting prompt");

        let args = prompt.arguments.unwrap();
        assert_eq!(args.len(), 1);

        let arg = &args[0];
        assert_eq!(arg.name, "name");
        assert_eq!(arg.description.as_ref().unwrap(), "Name of the person to greet");
        assert!(arg.required.unwrap());
    }
}
