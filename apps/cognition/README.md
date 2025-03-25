# RAG Server

A REST API server for Retrieval-Augmented Generation (RAG) built with Bioma.

## Features

- Document indexing and retrieval
- Chat with context from indexed documents
- Text and image embeddings generation
- Text reranking
- File upload support (single files and zip archives)

## RAG server example

[Agentic RAG Server](docs/rag_server.md)

## Quick Start

1. Start SurrealDB:

```bash
surreal start --no-banner --allow-all --bind 0.0.0.0:9123 --user root --pass root surrealkv://.output/bioma.db
```

2. Launch the server:

```bash
# Launch with default configuration
cargo run --release -p cognition --bin cognition-server

# Launch with custom tools configuration (stdio)
cargo run --release -p cognition --bin cognition-server assets/configs/rag_tools_config_server_stdio.json

# Launch with custom tools configuration (sse)
cargo run --release -p cognition --bin cognition-server assets/configs/rag_tools_config_server_sse.json
```

## Endpoints

For more information about the endpoint, please consult the [main README file.](../../README.md).

### Swagger-ui documentation

Documentation is provided in the dashboard and the `congition` server should be runing to visualize it.
