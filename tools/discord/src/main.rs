use clap::Parser;
use dotenv::dotenv;
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use text_splitter::MarkdownSplitter;
use tracing::{error, info};

use bioma_actor::prelude::*;
use bioma_llm::prelude::*;

use serenity::async_trait;
use serenity::builder::GetMessages;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::model::id::UserId;
use serenity::prelude::*;

use config::Args;
use user::UserActor;

mod config;
mod user;

#[derive(thiserror::Error, Debug)]
pub enum DiscordError {
    #[error("System error: {0}")]
    System(#[from] SystemActorError),
    #[error("Serenity error: {0}")]
    Serenity(#[from] SerenityError),
    #[error("Parse structured error: {0}")]
    ParseStructured(String),
}

impl ActorError for DiscordError {}

struct Handler {
    engine: Engine,
    chat: ActorId,
    _indexer: ActorId,
    retriever: ActorId,
    _embeddings: ActorId,
    _rerank: ActorId,
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        // Ignore messages from bots or self
        if msg.author.bot || msg.author.id == ctx.cache.current_user().id {
            return;
        }

        info!("Message received: {}", msg.content);

        // Get the author's actor for bioma requests
        let author_ctx = match self.get_author_user(&msg).await {
            Ok(ctx) => ctx,
            Err(e) => {
                error!("Error getting author context: {:?}", e);
                return;
            }
        };

        // Build conversation history
        let mut conversation = match self.build_conversation_history(&ctx, &msg).await {
            Ok(conv) => conv,
            Err(e) => {
                error!("Error building conversation history: {:?}", e);
                return;
            }
        };

        // Get the bot's own user ID from the context
        let bot_id = ctx.cache.current_user().id;

        let bot_mentioned = msg.mentions.iter().any(|user| user.id == bot_id);
        let bot_needs_to_respond = if bot_mentioned {
            true
        } else {
            match self.does_bot_need_to_respond(&ctx, &msg, &author_ctx, conversation.clone()).await {
                Ok(response) => response,
                Err(e) => {
                    error!("Error determining if bot needs to respond: {:?}", e);
                    false
                }
            }
        };

        // Check if bot needs to respond
        if bot_needs_to_respond {
            info!("Bot needs to respond, generating response...");

            // Retrieve context
            let context = match self.retrieve_context(&author_ctx, &conversation).await {
                Ok(ctx) => ctx,
                Err(e) => {
                    error!("Error retrieving context: {:?}", e);
                    "".to_string()
                }
            };

            // Insert context into system message if available
            if !context.is_empty() {
                if let Some(first_message) = conversation.first_mut() {
                    if first_message.role == MessageRole::System {
                        first_message.content = format!("---\nContext:\n{}\n\n{}", context, first_message.content);
                    }
                }
            }

            // Generate response with prepared conversation and context
            let llm_response = match self.generate_llm_response(&author_ctx, conversation).await {
                Ok(response) => response,
                Err(e) => {
                    error!("Error generating LLM response: {:?}", e);
                    return;
                }
            };

            if !llm_response.is_empty() {
                // Send the LLM-generated response to the channel
                // Message content must be under 2000 unicode code points.
                let splitter = MarkdownSplitter::new(1500);
                let chunks = splitter.chunks(&llm_response).collect::<Vec<&str>>();
                for chunk in chunks {
                    if let Err(why) = msg.channel_id.say(&ctx.http, chunk).await {
                        error!("Error sending message: {why:?}");
                    }
                }
            }
        } else if msg.content == "!ping" {
            if let Err(why) = msg.channel_id.say(&ctx.http, "Pong!").await {
                error!("Error sending message: {why:?}");
            }
        }
    }

    async fn ready(&self, _: Context, ready: Ready) {
        info!("{} is connected!", ready.user.name);
    }
}

#[derive(JsonSchema, Debug, Clone, Serialize, Deserialize)]
struct ShouldRespond {
    #[schemars(description = "Whether the bot should respond to the message")]
    should_respond: bool,
    #[schemars(description = "The reason for the bot's response")]
    reason: String,
}

impl Handler {
    async fn does_bot_need_to_respond(
        &self,
        ctx: &Context,
        msg: &Message,
        author_ctx: &ActorContext<UserActor>,
        mut conversation: Vec<ChatMessage>,
    ) -> Result<bool, DiscordError> {
        let channel = ctx.http.get_channel(msg.channel_id).await.map_err(|e| DiscordError::Serenity(e))?;
        let channel_name = channel.guild().map(|c| c.name).unwrap_or_else(|| "unknown-channel".to_string());

        let bot_id = ctx.cache.current_user().id.to_string();
        let bot_name = ctx.cache.current_user().name.clone();

        let system_prompt = format!(
            r#"
            ---
            Instructions:

            You are in a Discord server, in the group channel #{channel_name}. 
            This is a multi-user conversation where different people are participating.
            You are the bot named "{bot_name}" (ID: {bot_id}), and your role is to determine if the current user's message should be responded to by the you, the bot.

            Below is the recent message history leading up to the current user's message.
            Each message is prefixed with the username of the person who sent it.

            Determine if the current user's message is intended for the bot and should be responded to.
            ---
            "#
        );

        // Replace the current system's message with the system prompt
        if let Some(first_message) = conversation.first_mut() {
            if first_message.role == MessageRole::System {
                first_message.content = system_prompt;
            }
        }

        // Force a structured response
        let format = bioma_llm::chat::Schema::new::<ShouldRespond>();

        let chat_response = author_ctx
            .send_and_wait_reply::<Chat, ChatMessages>(
                ChatMessages {
                    messages: conversation.clone(),
                    restart: true,
                    persist: false,
                    stream: false,
                    format: Some(format),
                    tools: None,
                },
                &self.chat,
                SendOptions::builder().timeout(std::time::Duration::from_secs(600)).build(),
            )
            .await?;

        // Get the response content
        let response_message = chat_response.message.content;

        // Convert the response to ShouldRespond
        let should_respond = serde_json::from_str::<ShouldRespond>(&response_message)
            .map_err(|e| DiscordError::ParseStructured(e.to_string()))?;

        info!("Bot should respond: {}", should_respond.should_respond);
        info!("Reason: {}", should_respond.reason);

        Ok(should_respond.should_respond)
    }

    async fn generate_llm_response(
        &self,
        author_ctx: &ActorContext<UserActor>,
        conversation: Vec<ChatMessage>,
    ) -> Result<String, DiscordError> {
        info!("Conversation sent to LLM: {:#?}", &conversation);

        // Debug save the conversation to a file
        // let debug_file = self.engine.output_dir().join("discord-conversation.json");
        // let conversation_json = serde_json::to_string(&conversation).unwrap();
        // tokio::fs::write(debug_file, conversation_json).await.unwrap();

        let chat_response = author_ctx
            .send_and_wait_reply::<Chat, ChatMessages>(
                ChatMessages {
                    messages: conversation.clone(),
                    restart: true,
                    persist: false,
                    stream: false,
                    format: None,
                    tools: None,
                },
                &self.chat,
                SendOptions::builder().timeout(std::time::Duration::from_secs(600)).build(),
            )
            .await?;

        // Debug save the response to a file
        // let debug_file = self.engine.output_dir().join("discord-response.md");
        // tokio::fs::write(debug_file, chat_response.message.content.clone()).await.unwrap();

        // Get the response content
        let mut response_message = chat_response.message.content;

        // Remove <think></think> tags and their content using regex
        let think_tag_regex = Regex::new(r"<think>[\s\S]*?</think>").unwrap();
        response_message = think_tag_regex.replace_all(&response_message, "").to_string();

        // Trim any extra whitespace that might be left after removing tags
        response_message = response_message.trim().to_string();

        Ok(response_message)
    }

    async fn build_conversation_history(&self, ctx: &Context, msg: &Message) -> Result<Vec<ChatMessage>, DiscordError> {
        let bot_id = ctx.cache.current_user().id.to_string();
        let bot_name = ctx.cache.current_user().name.clone();

        // Fetch the last n messages in the channel
        let messages = match msg.channel_id.messages(&ctx.http, GetMessages::new().limit(5)).await {
            Ok(messages) => messages,
            Err(why) => {
                error!("Error fetching messages: {why:?}");
                vec![]
            }
        };

        // Replace user IDs with usernames in a more efficient way
        let re = Regex::new(r"<@!?(\d+)>").unwrap();

        // Function to replace mentions
        let replace_mentions = |text: &str, user_id_map: &HashMap<UserId, String>| {
            re.replace_all(text, |caps: &regex::Captures| {
                let user_id_str = &caps[1];
                if let Ok(user_id) = user_id_str.parse::<u64>() {
                    let user_id = UserId::new(user_id);
                    if let Some(name) = user_id_map.get(&user_id) { name.clone() } else { format!("<@{}>", user_id) }
                } else {
                    caps[0].to_string()
                }
            })
            .to_string()
        };

        // First, collect all user IDs that need to be fetched from all messages
        let mut user_id_map = HashMap::new();

        // Add bot to the map
        user_id_map.insert(UserId::new(bot_id.parse::<u64>().unwrap()), bot_name.clone());

        // Collect mentions from current message and history
        let mut all_texts = vec![&msg.content];
        for m in &messages {
            all_texts.push(&m.content);
        }

        // Find all unique user IDs mentioned
        let mut all_mentions = HashSet::new();
        for text in &all_texts {
            for caps in re.captures_iter(text) {
                if let Ok(user_id) = caps[1].parse::<u64>() {
                    all_mentions.insert(UserId::new(user_id));
                }
            }
        }

        // Fetch user information for all mentioned users from cache
        for user_id in all_mentions {
            if user_id.to_string() == bot_id {
                continue; // Already added bot
            } else if let Some(user) = ctx.cache.user(user_id) {
                info!("User from cache - ID: {}, Name: {}", user_id, user.name);
                user_id_map.insert(user_id, user.name.clone());
            }
        }

        // Build conversation history as ChatMessages
        let mut conversation = Vec::new();

        // System prompt with group channel context
        let channel = ctx.http.get_channel(msg.channel_id).await.map_err(|e| DiscordError::Serenity(e))?;

        let channel_name = channel.guild().map(|c| c.name).unwrap_or_else(|| "unknown-channel".to_string());
        let system_prompt = format!(
            r#"
            ---
            Instructions:

            You are in a Discord server, in the group channel #{channel_name}. 
            This is a multi-user conversation where different people are participating.
            You are the bot named "{bot_name}" (ID: {bot_id}), and your role is to assist users by answering questions and helping with tasks.

            Below is the recent message history leading up to the current user's message.
            Each message is prefixed with the username of the person who sent it.
            Respond naturally as "{bot_name}" and address users by their usernames when relevant.
            ---
            "#
        );
        conversation.push(ChatMessage {
            role: MessageRole::System,
            content: system_prompt,
            images: None,
            tool_calls: vec![],
        });

        // Add historical messages from multiple users, includes the current message
        for m in messages.iter().rev() {
            let content = replace_mentions(&m.content, &user_id_map);

            // Remove leading dot from username if present
            let author_name =
                if m.author.name.starts_with('.') { m.author.name.trim_start_matches('.') } else { &m.author.name };

            conversation.push(ChatMessage {
                role: if m.author.id.to_string() == bot_id { MessageRole::Assistant } else { MessageRole::User },
                content: format!("{}: {}", author_name, content),
                images: None,
                tool_calls: vec![],
            });
        }

        Ok(conversation)
    }

    async fn retrieve_context(
        &self,
        author_ctx: &ActorContext<UserActor>,
        messages: &Vec<ChatMessage>,
    ) -> Result<String, DiscordError> {
        // Extract user messages to form the query
        let query = messages
            .iter()
            .filter(|message| message.role == MessageRole::User)
            .map(|message| message.content.clone())
            .collect::<Vec<String>>()
            .join("\n");

        info!("Retrieval query: {:#?}", query);

        // Send retrieval request
        info!("Sending message to retriever actor");
        let retrieve_context = RetrieveContext {
            query: RetrieveQuery::Text(query),
            limit: 5,
            threshold: 0.0,
            sources: vec!["/bioma".to_string()],
        };

        let retrieved = author_ctx
            .send_and_wait_reply::<Retriever, RetrieveContext>(
                retrieve_context,
                &self.retriever,
                SendOptions::builder().timeout(std::time::Duration::from_secs(200)).build(),
            )
            .await?;

        // Format the retrieved context
        let context = retrieved.to_markdown();

        info!("Retrieved context: {}", if context.is_empty() { "None" } else { "Found" });

        Ok(context)
    }

    // Get or create a user actor context for the author of the message
    async fn get_author_user(&self, msg: &Message) -> Result<ActorContext<UserActor>, DiscordError> {
        let user_actor_id = ActorId::of::<UserActor>(format!("/discord/user/{}", msg.author.id.to_string()));

        // Get or create the user actor
        let (user_ctx, _user_actor) = Actor::spawn(
            self.engine.clone(),
            user_actor_id.clone(),
            UserActor {},
            SpawnOptions::builder().exists(SpawnExistsOptions::Restore).build(),
        )
        .await?;

        Ok(user_ctx)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();

    // Initialize tracing
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let args = Args::parse();
    let config = args.load_config()?;

    // Initialize engine
    let engine = Engine::connect(config.engine.clone()).await?;

    // Spawn main actors and their relays
    let mut actor_handles = Vec::new();

    // Indexer setup
    let indexer_id = ActorId::of::<Indexer>("/discord/indexer");
    let indexer = Indexer::default();
    let (mut indexer_ctx, mut indexer_actor) = Actor::spawn(
        engine.clone(),
        indexer_id.clone(),
        indexer,
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await?;

    let indexer_handle = tokio::spawn(async move {
        if let Err(e) = indexer_actor.start(&mut indexer_ctx).await {
            error!("Indexer actor error: {}", e);
        }
    });
    actor_handles.push(indexer_handle);

    // Retriever setup
    let retriever_id = ActorId::of::<Retriever>("/discord/retriever");
    let (mut retriever_ctx, mut retriever_actor) = Actor::spawn(
        engine.clone(),
        retriever_id.clone(),
        Retriever::default(),
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await?;

    let retriever_handle = tokio::spawn(async move {
        if let Err(e) = retriever_actor.start(&mut retriever_ctx).await {
            error!("Retriever actor error: {}", e);
        }
    });
    actor_handles.push(retriever_handle);

    // Embeddings setup
    let embeddings_id = ActorId::of::<Embeddings>("/discord/embeddings");
    let (mut embeddings_ctx, mut embeddings_actor) = Actor::spawn(
        engine.clone(),
        embeddings_id.clone(),
        Embeddings::default(),
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await?;

    let embeddings_handle = tokio::spawn(async move {
        if let Err(e) = embeddings_actor.start(&mut embeddings_ctx).await {
            error!("Embeddings actor error: {}", e);
        }
    });
    actor_handles.push(embeddings_handle);

    // Rerank setup
    let rerank_id = ActorId::of::<Rerank>("/discord/rerank");
    let (mut rerank_ctx, mut rerank_actor) = Actor::spawn(
        engine.clone(),
        rerank_id.clone(),
        Rerank::default(),
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await?;

    let rerank_handle = tokio::spawn(async move {
        if let Err(e) = rerank_actor.start(&mut rerank_ctx).await {
            error!("Rerank actor error: {}", e);
        }
    });
    actor_handles.push(rerank_handle);

    // Chat setup
    let chat_id = ActorId::of::<Chat>("/discord/chat");
    let (mut chat_ctx, mut chat_actor) = Actor::spawn(
        engine.clone(),
        chat_id.clone(),
        Chat::builder()
            .model(config.chat_model.clone())
            .endpoint(config.chat_endpoint.clone())
            // .generation_options(ollama_rs::generation::options::GenerationOptions::default().num_predict(500))
            .build(),
        SpawnOptions::builder().exists(SpawnExistsOptions::Reset).build(),
    )
    .await?;

    let chat_handle = tokio::spawn(async move {
        if let Err(e) = chat_actor.start(&mut chat_ctx).await {
            error!("Chat actor error: {}", e);
        }
    });
    actor_handles.push(chat_handle);

    // Bot handler
    let handler = Handler {
        engine,
        chat: chat_id,
        _indexer: indexer_id,
        retriever: retriever_id,
        _embeddings: embeddings_id,
        _rerank: rerank_id,
    };

    let token = env::var("DISCORD_TOKEN").expect("Expected a token in the environment");
    let intents = GatewayIntents::GUILD_MESSAGES | GatewayIntents::DIRECT_MESSAGES | GatewayIntents::MESSAGE_CONTENT;

    let mut client = Client::builder(&token, intents).event_handler(handler).await.expect("Err creating client");

    if let Err(why) = client.start().await {
        error!("Client error: {why:?}");
    }

    Ok(())
}
