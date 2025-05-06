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

## Example Client

A client example is provided to demonstrate how to interact with the RAG MCP server:

```bash
# Run client with stdio transport
RUST_LOG=debug cargo run -p rag_mcp --example client stdio

# Run client with SSE transport
RUST_LOG=debug cargo run -p rag_mcp --example client sse --endpoint http://127.0.0.1:8090

# Run client with WebSocket transport
RUST_LOG=debug cargo run -p rag_mcp --example client ws --endpoint ws://127.0.0.1:9090

# Run client with Streamable transport
RUST_LOG=debug cargo run -p rag_mcp --example client streamable --endpoint http://127.0.0.1:7090
```

The client example demonstrates:

- Connecting to the server using different transport options
- Document ingestion and indexing
- Query retrieval and embedding generation
- Source management and deletion
- RAG-augmented LLM response generation

## Testing

Run all tools tests:

```bash
RUST_LOG=info cargo test --package rag_mcp --bin rag_mcp -- tools --show-output
```

Module-specific tests can also be run:

```bash
# Example: Run tests for a specific module
RUST_LOG=info cargo test --package rag_mcp --bin rag_mcp -- module_name --show-output
```

## Tools

The server provides the following MCP tools:

- **index**: Index documents for retrieval
- **retrieve**: Retrieve documents based on queries
- **embed**: Generate text and image embeddings
- **rerank**: Rerank retrieved documents based on relevance
- **upload**: Upload files for processing
- **sources**: Manage document sources
- **delete**: Delete indexed content
