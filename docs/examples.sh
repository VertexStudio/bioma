#!/bin/bash

# RAG Server API Examples
# =====================
# Server runs on localhost:5766
# Ollama runs on localhost:11434

# Server Management
# ----------------
# Reset the RAG engine
curl -X POST http://localhost:5766/reset

# Document Management
# ----------------
# Index files using glob patterns
curl -X POST http://localhost:5766/index \
    -H "Content-Type: application/json" \
    -d '{"globs": ["/Users/rozgo/BiomaAI/bioma/bioma_actor/**/*.rs"], "chunk_capacity": {"start": 500, "end": 2000}, "chunk_overlap": 200}'

# Upload a single file
curl -X POST http://localhost:5766/upload \
    -F 'file=@./path/to/file.md' \
    -F 'metadata={"path": "dest/path/file.md"};type=application/json'

# Upload a zip archive
curl -X POST http://localhost:5766/upload \
    -F 'file=@./archive.zip' \
    -F 'metadata={"path": "dest/path/archive.zip"};type=application/json'

# Search and Retrieval
# ----------------
# Retrieve context
curl -X POST http://localhost:5766/retrieve \
    -H "Content-Type: application/json" \
    -d '{
        "type": "Text",
        "query": "What is Bioma?",
        "threshold": 0.0,
        "limit": 10,
        "source": ".*"
    }'

# Ask a question (RAG-enhanced)
curl -X POST http://localhost:5766/ask \
    -H "Content-Type: application/json" \
    -d '{
    "model": "llama3.2",
    "messages": [
        {
            "role": "user",
            "content": "What is Bioma?"
        }
    ]
}'

# Embedding Operations
# ----------------
# Generate embeddings
curl -X POST http://localhost:5766/embed \
    -H "Content-Type: application/json" \
    -d '{
        "model": "nomic-embed-text",
        "input": [
            "Why is the sky blue?",
            "Why is the grass green?"
        ]
    }'

# Rerank texts
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

# Chat completion (compatible with Ollama)
curl -X POST http://localhost:5766/chat \
    -H "Content-Type: application/json" \
    -d '{
        "model": "llama3.2",
        "messages": [
            {
                "role": "user",
                "content": "Why is the sky blue?"
            }
        ],
        "fetch_tools": false
    }'

# Structured ask with schema (compatible with Ollama)
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

# Ollama Endpoints (running on port 11434)
# ----------------
# Generate a response
curl -X POST http://localhost:11434/api/generate \
    -H "Content-Type: application/json" \
    -d '{
        "model": "qwen2.5-coder:32b-instruct-q5_K_M",
        "prompt": "Why is the sky blue?"
    }'

# Chat completion
curl -X POST http://127.0.0.1:11434/api/generate \
    -H "Content-Type: application/json" \
    -d '{
        "model": "qwen2.5-coder:32b-instruct-q5_K_M",
        "messages": [
            {
                "role": "user",
                "content": "Why is the sky blue?"
            }
        ]
    }'

curl http://127.0.0.1:11434/v1/chat/completions \
    -H "Content-Type: application/json" \
    -d '{
        "model": "qwen2.5-coder:32b-instruct-q5_K_M",
        "messages": [
            {
                "role": "system",
                "content": "You are Mistral.rs, an AI assistant."
            },
            {
                "role": "user",
                "content": "Say hello."
            }
        ]
    }'

# MCP Server Inspector
npx @modelcontextprotocol/inspector /Users/rozgo/BiomaAI/bioma/target/release/examples/mcp_server --log-file /Users/rozgo/BiomaAI/bioma/.output/mcp_server-inspector.log