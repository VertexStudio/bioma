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
    -d '{"globs": ["bioma_actor/**/*.rs"]}'

# Upload a single file
curl -X POST http://localhost:5766/upload \
    -X POST \
    -F 'file=@./path/to/file.md' \
    -F 'metadata={"path": "dest/path/file.md"};type=application/json'

# Upload a zip archive
curl -X POST http://localhost:5766/upload \
    -X POST \
    -F 'file=@./archive.zip' \
    -F 'metadata={"path": "archive.zip"};type=application/json'

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
    -d '{"query": "What is Bioma?"}'

# Embedding Operations
# ----------------
# Generate embeddings
curl -X POST http://localhost:5766/api/embed \
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
curl -X POST http://localhost:5766/api/chat \
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

# Ollama Endpoints (running on port 11434)
# ----------------
# Generate a response
curl -X POST http://localhost:11434/api/generate \
    -H "Content-Type: application/json" \
    -d '{
        "model": "llama3.2",
        "prompt": "Why is the sky blue?"
    }'

# Chat completion
curl -X POST http://localhost:11434/api/chat \
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
