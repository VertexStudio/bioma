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
- **Flexible Transport**: Supports WebSocket, SSE, and stdio transports for versatile integration options
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
                "--log-file",
                "/Users/rozgo/BiomaAI/bioma/.output/mcp_server-claude.log",
                "--transport",
                "stdio"
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

MCP client with stdio transport:

```
cargo run --release -p bioma_tool --example mcp_client -- stdio target/release/examples/mcp_server
```

MCP client with SSE transport:

```
cargo run --release -p bioma_tool --example mcp_client -- sse --endpoint http://127.0.0.1:8090
```

MCP server:

```
cargo build --release -p bioma_tool --example mcp_server
```

MCP server with SSE transport:

```
cargo run --release -p bioma_tool --example mcp_server -- --transport sse --url 127.0.0.1:8090
```

Inspect example server:

```
npx github:VertexStudio/inspector#feature/ui-ux ./target/release/examples/mcp_server --log-file .output/mcp_server-inspector.log
```

Connecting to docker server:

```
cargo run --release -p bioma_tool --example mcp_client -- stdio docker run -i --rm --mount "type=bind,src=/Users/rozgo/BiomaAI/bioma,dst=/data/BiomaAI,ro" mcp/filesystem /data/BiomaAI
```
