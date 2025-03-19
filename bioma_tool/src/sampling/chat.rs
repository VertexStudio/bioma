use crate::sampling::{SamplingDef, SamplingError};
use crate::schema::{CreateMessageRequest, CreateMessageResult, Role};
use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use ollama_rs::models::ModelOptions;
use serde::Serialize;

/// A sampling implementation using the Chat actor
#[derive(Clone, Debug, Serialize)]
pub struct ChatSampling {
    #[serde(skip)]
    engine: Engine,
}

impl ChatSampling {
    /// Creates a new ChatSampling instance
    pub fn new(engine: Engine) -> Self {
        Self { engine }
    }
}

impl SamplingDef for ChatSampling {
    async fn create_message(&self, request: CreateMessageRequest) -> Result<CreateMessageResult, SamplingError> {
        // Create a new Chat actor for this request
        let chat_id = ActorId::of::<Chat>("/llm/sampling");

        // Extract model preference or use default
        let model = request
            .params
            .model_preferences
            .and_then(|prefs| prefs.hints.and_then(|hints| hints.first().and_then(|hint| hint.name.clone())))
            .unwrap_or_else(|| "llama3.2".into());

        // Create the Chat actor
        let chat = Chat::builder().model(model.clone().into()).build();

        let (mut chat_ctx, mut chat_actor) =
            Actor::spawn(self.engine.clone(), chat_id.clone(), chat, SpawnOptions::default())
                .await
                .map_err(|e| SamplingError::Execution(e.to_string()))?;

        // Convert SamplingMessage to ChatMessages
        let messages = request
            .params
            .messages
            .iter()
            .map(|msg| {
                let role = match msg.role {
                    Role::User => MessageRole::User,
                    Role::Assistant => MessageRole::Assistant,
                };
                ChatMessage {
                    role,
                    content: serde_json::to_string(&msg.content).unwrap_or_default(),
                    images: None,
                    tool_calls: Vec::new(),
                }
            })
            .collect::<Vec<_>>();

        // Set sampling parameters using ModelOptions
        let mut model_options = ModelOptions::default();
        if let Some(temp) = request.params.temperature {
            model_options = model_options.temperature(temp as f32);
        }
        // Convert max_tokens i64 to i32 for num_predict
        model_options = model_options.num_predict(request.params.max_tokens as i32);

        // Create ChatMessages request
        let chat_messages = ChatMessages::builder().messages(messages).restart(true).options(model_options).build();

        // Use a simpler approach by just running the actor in the current task
        chat_actor
            .start(&mut chat_ctx)
            .await
            .map_err(|e| SamplingError::Execution(format!("Failed to start chat actor: {}", e)))?;

        // Send the message directly and wait for response
        let response = chat_ctx
            .send_and_wait_reply::<Chat, _>(chat_messages, &chat_id, SendOptions::default())
            .await
            .map_err(|e| SamplingError::Execution(format!("Failed to get response: {}", e)))?;

        // Convert ChatMessageResponse to CreateMessageResult
        let result = CreateMessageResult {
            meta: None,
            content: serde_json::from_str(&response.message.content)
                .unwrap_or(serde_json::Value::String(response.message.content)),
            model,
            role: Role::Assistant,
            stop_reason: Some("stop".to_string()),
        };

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{CreateMessageRequestParams, SamplingMessage};

    #[tokio::test]
    async fn test_chat_sampling() {
        // Initialize actor engine
        let engine = Engine::test().await.unwrap();

        // Create a ChatSampling instance
        let sampling = ChatSampling::new(engine);

        // Create a test request
        let request = CreateMessageRequest {
            method: "sampling.create_message".to_string(),
            params: CreateMessageRequestParams {
                messages: vec![SamplingMessage {
                    content: serde_json::Value::String("What is the capital of France?".to_string()),
                    role: Role::User,
                }],
                max_tokens: 128,
                temperature: Some(0.7),
                model_preferences: None,
                include_context: None,
                metadata: None,
                stop_sequences: None,
                system_prompt: None,
            },
        };

        // Call create_message and check the result
        let result = sampling.create_message(request).await.unwrap();

        // Assert on the result
        assert_eq!(result.role, Role::Assistant);
        // Note: In a real test, we'd use a mock LLM to ensure consistent responses
    }
}
