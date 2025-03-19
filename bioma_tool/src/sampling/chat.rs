use crate::sampling::{SamplingDef, SamplingError};
use crate::schema::{CreateMessageRequest, CreateMessageResult, Role};
use bioma_actor::prelude::*;
use bioma_llm::prelude::*;
use ollama_rs::models::ModelOptions;
use serde::Serialize;
use tracing;

/// A sampling implementation using the Chat actor
#[derive(Clone, Debug, Serialize)]
pub struct ChatSampling {
    #[serde(skip)]
    engine: Engine,
    #[serde(skip)]
    chat_id: ActorId,
}

impl ChatSampling {
    /// Creates a new ChatSampling instance
    pub async fn new() -> Result<Self, SamplingError> {
        let engine = Engine::test().await.map_err(|e| SamplingError::Execution(e.to_string()))?;

        // Create a single Chat actor for all requests
        let chat_id = ActorId::of::<Chat>("/llm/sampling");
        let chat = Chat::default();

        // Spawn the Chat actor
        let (mut chat_ctx, mut chat_actor) =
            Actor::spawn(engine.clone(), chat_id.clone(), chat, SpawnOptions::default())
                .await
                .map_err(|e| SamplingError::Execution(e.to_string()))?;

        // Start the Chat actor in a separate task
        let _chat_handle = tokio::spawn(async move {
            if let Err(e) = chat_actor.start(&mut chat_ctx).await {
                tracing::error!("Chat actor error: {}", e);
            }
        });

        Ok(Self { engine, chat_id })
    }
}

impl SamplingDef for ChatSampling {
    async fn create_message(&self, request: CreateMessageRequest) -> Result<CreateMessageResult, SamplingError> {
        // Create a relay actor to send messages
        let relay_id = ActorId::of::<Relay>("/relay");
        let (relay_ctx, _relay_actor) =
            Actor::spawn(self.engine.clone(), relay_id.clone(), Relay, SpawnOptions::default())
                .await
                .map_err(|e| SamplingError::Execution(e.to_string()))?;

        // Set sampling parameters using ModelOptions
        let mut model_options = ModelOptions::default();
        if let Some(temp) = request.params.temperature {
            model_options = model_options.temperature(temp as f32);
        }
        // Convert max_tokens i64 to i32 for num_predict
        model_options = model_options.num_predict(request.params.max_tokens as i32);

        // Add stop sequences if provided
        if let Some(stop_seqs) = request.params.stop_sequences {
            model_options = model_options.stop(stop_seqs);
        }

        // Create messages list, potentially with system prompt
        let mut messages = Vec::new();

        // Add system prompt as first message if provided
        if let Some(system_prompt) = request.params.system_prompt {
            messages.push(ChatMessage {
                role: MessageRole::System,
                content: system_prompt,
                images: None,
                tool_calls: Vec::new(),
            });
        }

        // Add user and assistant messages
        messages.extend(request.params.messages.iter().map(|msg| {
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
        }));

        // TODO: Handle model_preferences to select appropriate model
        // This would typically influence model selection, but we're using a
        // fixed model via the Chat actor currently

        // Create ChatMessages request
        let chat_messages = ChatMessages::builder().messages(messages).restart(true).options(model_options).build();

        // Send the message via the relay actor and wait for response
        let response = relay_ctx
            .send_and_wait_reply::<Chat, ChatMessages>(
                chat_messages,
                &self.chat_id,
                SendOptions::builder().timeout(std::time::Duration::from_secs(60)).build(),
            )
            .await
            .map_err(|e| SamplingError::Execution(format!("Failed to get response: {}", e)))?;

        let role = match response.message.role {
            MessageRole::Assistant => Role::Assistant,
            MessageRole::User => Role::User,
            _ => {
                return Err(SamplingError::Execution(format!("Unsupported message role: {:?}", response.message.role)));
            }
        };

        // Convert ChatMessageResponse to CreateMessageResult
        let result = CreateMessageResult {
            meta: None,
            content: serde_json::from_str(&response.message.content)
                .unwrap_or(serde_json::Value::String(response.message.content)),
            model: response.model,
            role,
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
        // Create a ChatSampling instance
        let sampling = ChatSampling::new().await.unwrap();

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
