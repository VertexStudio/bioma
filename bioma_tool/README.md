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
- **Flexible Transport**: Supports both WebSocket and stdio transports for versatile integration options
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
            "command": "/Users/rozgo/BiomaAI/bioma/target/debug/examples/mcp_server",
            "args": [
                "--log-file",
                "/Users/rozgo/BiomaAI/bioma/.output/mcp_server.log",
                "--transport",
                "stdio"
            ]
        }
    }
}
```

Generate the schema.es from MCP schema.json
```
schemafy-cli src | rustfmt | tee src/schema.rs
```

