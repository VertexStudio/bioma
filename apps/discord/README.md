# Bioma Discord Bot

A Discord bot built with Rust that uses the Bioma actor system to provide AI-powered assistance in Discord channels.

## Features

- Responds to direct mentions and contextually relevant messages
- Retrieves information from a knowledge base to provide accurate answers
- Maintains conversation history for context-aware responses
- Handles multi-user conversations in Discord channels

## Prerequisites

- Rust and Cargo (latest stable version)
- Discord account with developer access
- Bioma actor system running locally or remotely

## Setup

### 1. Create a Discord Bot

1. Go to the [Discord Developer Portal](https://discord.com/developers/applications)
2. Click "New Application" and give your application a name
3. Navigate to the "Bot" tab and click "Add Bot"
4. Under the "Privileged Gateway Intents" section, enable:
   - MESSAGE CONTENT INTENT
   - SERVER MEMBERS INTENT
   - PRESENCE INTENT
5. Click "Reset Token" to generate a new token (or copy the existing one)
6. **Important**: Save this token securely - it will only be shown once!

### 2. Invite the Bot to Your Server

1. In the Developer Portal, go to the "OAuth2" → "URL Generator" tab
2. Select the following scopes:
   - `bot`
   - `applications.commands`
3. Select the following bot permissions:
   - Read Messages/View Channels
   - Send Messages
   - Read Message History
4. Copy the generated URL and open it in your browser
5. Select the server you want to add the bot to and authorize it

### 3. Configure Environment Variables

Create a `.env` file in the project root with the following variables:

```
DISCORD_TOKEN=your_discord_token_here
RUST_LOG=info
```

### 4. Build and Run

1. Make sure the Bioma actor system is running (default: `ws://0.0.0.0:9123`)
2. Run the Discord bot from the workspace root:

```bash
# Run with default configuration
cargo run --release -p discord

# Run with a specific configuration file
cargo run --release -p discord -- .output/discord_config_server.json
```

The `-p discord` flag specifies the package to run within the workspace.

## Configuration

The bot can be configured using a JSON file with the following structure:

```json
{
  "engine": {
    "endpoint": "ws://0.0.0.0:9123",
    "namespace": "dev",
    "database": "bioma",
    "username": "root",
    "output_dir": ".output",
    "local_store_dir": ".output/store",
    "hf_cache_dir": ".output/cache/huggingface/hub"
  },
  "chat_endpoint": "http://0.0.0.0:11434",
  "chat_model": "llama3.2-vision",
  "chat_prompt": "You are, Bioma, a helpful assistant. Your creator is Vertex Studio, a games and simulation company. Format your response in markdown. Use the following context to answer the user's query:\n\n",
  "chat_messages_limit": 10,
  "chat_context_length": 4096,
  "retrieve_limit": 5
}
```

All configuration fields are optional and will use default values if not specified:

| Field                    | Default                                  | Description                                                 |
| ------------------------ | ---------------------------------------- | ----------------------------------------------------------- |
| `engine.endpoint`        | `memory`                                 | Endpoint for the Bioma actor system                         |
| `engine.namespace`       | `dev`                                    | Namespace in the database                                   |
| `engine.database`        | `bioma`                                  | Database name                                               |
| `engine.username`        | `root`                                   | Database username                                           |
| `engine.password`        | `root`                                   | Database password                                           |
| `engine.output_dir`      | `.output`                                | Output directory for artifacts                              |
| `engine.local_store_dir` | `.output/store`                          | Local file system path for object store                     |
| `engine.hf_cache_dir`    | `.output/cache/huggingface/hub`          | HuggingFace cache directory                                 |
| `chat_endpoint`          | `http://0.0.0.0:11434`                   | HTTP endpoint for the chat service                          |
| `chat_model`             | `llama3.2-vision`                        | Model to use for chat responses                             |
| `chat_prompt`            | `You are, Bioma, a helpful assistant...` | System prompt for the chat model                            |
| `chat_messages_limit`    | `10`                                     | Maximum number of messages to keep in conversation history  |
| `chat_context_length`    | `4096`                                   | Maximum context length for the chat model                   |
| `retrieve_limit`         | `5`                                      | Maximum number of items to retrieve from the knowledge base |

## Troubleshooting

- **Bot doesn't respond**: Ensure the bot has the correct permissions in the Discord channel
- **Connection errors**: Check that the Bioma actor system is running and accessible
- **Token errors**: Verify your Discord token is correct and properly set in the `.env` file
