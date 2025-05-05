# RAG MCP Server

A Model Context Protocol (MCP) server implementation for Retrieval-Augmented Generation (RAG).

## Features

- Document indexing and retrieval
- Vector embeddings generation
- Text reranking
- File upload support (single files and zip archives)
- Document source management
- Delete functionality for indexed content

## Transport Options

The server supports multiple transport configurations:

- **Stdio**: Standard input/output for local usage
- **SSE**: Server-Sent Events for web applications
- **WebSockets**: Bidirectional communication
- **Streamable**: HTTP endpoint with configurable response type (JSON or SSE)

## Quick Start

1. Build the server:

```bash
cargo build --release -p rag_mcp
```

2. Run with default configuration (stdio transport):

```bash
cargo run --release -p rag_mcp
```

3. Run with custom transport:

```bash
# SSE transport
cargo run --release -p rag_mcp -- --sse --endpoint 127.0.0.1:8090

# WebSocket transport
cargo run --release -p rag_mcp -- --ws --endpoint 127.0.0.1:9090

# Streamable transport with JSON response
cargo run --release -p rag_mcp -- --streamable --endpoint 127.0.0.1:7090 --response-type json
```

## Command Line Options

- `--log-file, -l`: Path to log file (default: "rag_mcp_server.log")
- `--base-dir, -b`: Base directory for file operations (default: ".")
- `--page-size, -p`: Number of items per page (default: 20)
- `--db-endpoint`: Database endpoint (default: "memory")

## Tools

The server provides the following MCP tools:

- **index**: Index documents for retrieval
- **retrieve**: Retrieve documents based on queries
- **embed**: Generate text and image embeddings
- **rerank**: Rerank retrieved documents based on relevance
- **upload**: Upload files for processing
- **sources**: Manage document sources
- **delete**: Delete indexed content
