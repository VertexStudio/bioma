# Bioma Tool

A Rust implementation of the [Model Context Protocol (MCP)](https://modelcontextprotocol.io).

## What is Model Context Protocol?

Model Context Protocol (MCP) is an open protocol designed to enable seamless integration between Large Language Model (LLM) applications and external data sources and tools. It provides a standardized way for LLMs to:

- Access external data sources
- Execute tools and commands
- Maintain context across interactions
- Handle real-time updates and notifications

### Key Features

- **Standardized Communication**: Built on JSON-RPC 2.0, providing a reliable and well-understood foundation for client-server communication
- **Flexible Transport**: Supports WebSocket, SSE, stdio, and streamable transports for versatile integration options
- **Tool Integration**: Define and execute custom tools with structured input/output schemas
- **Resource Management**: Access and manipulate external resources with a unified interface
- **Progress Tracking**: Monitor long-running operations with built-in progress notifications
- **Memory Management**: Store and retrieve contextual information across interactions

### Built-in Tools

The implementation includes several built-in tools:

- `echo`: Simple tool for testing and demonstration
- `fetch`: Retrieve and process web content with support for robots.txt compliance
- `memory`: Key-value storage for maintaining context across interactions

### Usage

To start a basic MCP server:

Configure the claude_config.json file. In macOS, it is located in the following path:

```
~/Library/Application Support/Claude/claude_desktop_config.json
```

Add the following to the file:

```
{
    "mcpServers": {
        "bioma-tool": {
            "command": "/Users/rozgo/BiomaAI/bioma/target/release/examples/mcp_server",
            "args": [
                "stdio",
                "--log-file",
                "/Users/rozgo/BiomaAI/bioma/.output/mcp_server-claude.log"
            ]
        }
    }
}
```

Test servers using [mcp-cli](https://github.com/wong2/mcp-cli):

```
npx @wong2/mcp-cli
```

Generate the schema.es from MCP schema.json

```
schemafy-cli src | rustfmt | tee src/schema.rs
```

Examples:

#### MCP Client

MCP client runs with stdio transport by default if no configuration is provided.

With JSON configuration:

```
cargo run --release -p bioma_mcp --example mcp_client --config assets/configs/mcp_client_config.json
```

With command-line options:

```
# Stdio transport with custom timeout
cargo run --release -p bioma_mcp --example mcp_client stdio --request-timeout 30

# SSE transport
cargo run --release -p bioma_mcp --example mcp_client sse --endpoint http://127.0.0.1:8090 --request-timeout 45

# WebSocket transport
cargo run --release -p bioma_mcp --example mcp_client ws --endpoint ws://127.0.0.1:9090

# Streamable transport
cargo run --release -p bioma_mcp --example mcp_client streamable --endpoint http://127.0.0.1:7090 --request-timeout 30
```

Example configuration file (`assets/configs/mcp_client_config.json`):

```json
{
  "servers": [
    {
      "name": "stdio-server",
      "transport": "stdio",
      "command": "target/release/examples/mcp_server",
      "args": ["stdio"],
      "request_timeout": 20
    },
    {
      "name": "sse-server",
      "transport": "sse",
      "endpoint": "http://127.0.0.1:8090"
    },
    {
      "name": "websocket-server",
      "transport": "ws",
      "endpoint": "ws://127.0.0.1:9090",
      "request_timeout": 45
    },
    {
      "name": "streamable-server",
      "transport": "streamable",
      "endpoint": "http://127.0.0.1:7090",
      "request_timeout": 30
    }
  ]
}
```

#### MCP Server

Building the server:

```
cargo build --release -p bioma_mcp --example mcp_server
```

With stdio transport:

```
cargo run --release -p bioma_mcp --example mcp_server stdio
```

With SSE transport:

```
cargo run --release -p bioma_mcp --example mcp_server sse --endpoint 127.0.0.1:8090
```

With WebSocket transport:

```
cargo run --release -p bioma_mcp --example mcp_server ws --endpoint 127.0.0.1:9090
```

With Streamable transport:

```
cargo run --release -p bioma_mcp --example mcp_server streamable --endpoint 127.0.0.1:7090 --response-type json --allowed-origins 0.0.0.0
```

The streamable transport supports two response types:

- `json`: Returns JSON responses (default)
- `sse`: Returns Server-Sent Events responses

You can also specify multiple allowed origins for CORS by separating them with commas:

```
cargo run --release -p bioma_mcp --example mcp_server streamable --endpoint 127.0.0.1:7090 --allowed-origins localhost,example.com
```

#### Other Examples

Inspect example server:

```
npx github:VertexStudio/inspector#feature/ui-ux ./target/release/examples/mcp_server --log-file .output/mcp_server-inspector.log
```

Connecting to docker server:

```
cargo run --release -p bioma_mcp --example mcp_client stdio docker run -i --rm --mount "type=bind,src=/Users/rozgo/BiomaAI/bioma,dst=/data/BiomaAI,ro" mcp/filesystem /data/BiomaAI
```
