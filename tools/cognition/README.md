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

# Launch with custom tools configuration
cargo run --release -p cognition --bin cognition-server assets/configs/rag_tools_config_server.json
```

## Endpoints

### Launch the server:

```bash
cargo run --release -p cognition --bin cognition-server
```

### Reset the engine:

```bash
curl -X POST http://localhost:5766/reset
```

### Upload files:

```bash
# Upload a single file
curl -X POST http://localhost:5766/upload \
    -F 'file=@./path/to/file.md' \
    -F 'metadata={"path": "dest/path/file.md"};type=application/json'

# Upload a zip archive
curl -X POST http://localhost:5766/upload \
    -F 'file=@./archive.zip' \
    -F 'metadata={"path": "archive.zip"};type=application/json'
```

### Index files:

```bash
curl -X POST http://localhost:5766/index \
    -H "Content-Type: application/json" \
    -d '{"globs": ["./path/to/files/**/*.rs"], "chunk_capacity": {"start": 500, "end": 2000}, "chunk_overlap": 200}'
```

### Retrieve context:

```bash
curl -X POST http://localhost:5766/retrieve \
    -H "Content-Type: application/json" \
    -d '{
        "type": "Text",
        "query": "What is Bioma?",
        "threshold": 0.0,
        "limit": 10,
        "source": ".*"
    }'
```

### Generate embeddings:

```bash
# Text embeddings example
curl -X POST http://localhost:5766/embed \
    -H "Content-Type: application/json" \
    -d '{
        "model": "nomic-embed-text",
        "input": [
            "Why is the sky blue?",
            "Why is the grass green?"
        ]
    }'

# Image embeddings example (base64 encoded images)
curl -X POST http://localhost:5766/embed \
    -H "Content-Type: application/json" \
    -d '{
        "model": "nomic-embed-vision",
        "input": "iVBORw0KGgoAAAANSUhEUgAAAAoAAAAKCAIAAAACUFjqAAABRklEQVR4nAA2Acn+A2ql2+Vv1LF7X3Mw2i9cMEBUs0/l0C6/irfF6wPqowTw0ORE00EZ/He1x+LwZ3nDwaZVNIgn6FI8KQabKikArD0j4g6LU2Mz9DpsAgnYGy6195whWQQ4XIk1a74tA98BtQfyE3oQkaA/uufBkIegK+TH6LMh/O44hIio5wAw4umxtkxZNCIf35A4YNshDwNeeHFnHP0YUSelrm8DMioFvjc7QOcZmEBw/pv+SXEH2G+O0ZdiHDTb6wnhAcRk1rkuJLwy/d7DDKTgqOflV5zk7IBgmz0f8J4o5gA4yb3rYzzUyLRXS0bY40xnoY/rtniWFdlrtSHkR/0A1ClG/qVWNyD1CXVkxE4IW5Tj+8qk1sD42XW6TQpPAO7NhmcDxDz092Q2AR8XYKPa1LPkGberOYArt0gkbQEAAP//4hWZNZ4Pc4kAAAAASUVORK5CYII="
    }'
```

### Rerank texts:

```bash
curl -X POST http://localhost:5766/rerank \
    -H "Content-Type: application/json" \
    -d '{
        "query": "What is Deep Learning?",
        "texts": [
            "Deep Learning is learning under water",
            "Deep learning is a branch of machine learning"
        ],
        "raw_scores": false
    }'
```

### Chat completion:

```bash
curl -X POST http://localhost:5766/chat \
    -H "Content-Type: application/json" \
    -d '{
        "model": "llama3.2",
        "messages": [
            {
                "role": "user",
                "content": "Why is the sky blue?"
            }
        ]
    }'
```

### Ask a question:

```bash
curl -X POST http://localhost:5766/ask \
    -H "Content-Type: application/json" \
    -d '{
        "model": "llama3.2",
        "messages": [
            {
                "role": "user",
                "content": "Tell me about Puerto Rico."
            }
        ],
        "format": {
            "title": "PuertoRicoInfo",
            "type": "object",
            "required": [
                "name",
                "capital",
                "languages"
            ],
            "properties": {
                "name": {
                    "description": "Name of the territory",
                    "type": "string"
                },
                "capital": {
                    "description": "Capital city",
                    "type": "string"
                },
                "languages": {
                    "description": "Official languages spoken",
                    "type": "array",
                    "items": {
                        "type": "string"
                    }
                }
            }
        }
    }'
```

### Delete indexed sources:

```bash
curl -X POST http://localhost:5766/delete_source \
    -H "Content-Type: application/json" \
    -d '{"sources": ["path/to/source1", "path/to/source2"]}'
```  

### Swagger-ui documentation

To run the Swagger-ui page, please use:

```bash
docker compose up swagger-ui
```

Then, open the browser at `http://localhost:80`. 
