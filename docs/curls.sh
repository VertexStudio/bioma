#! /bin/bash


# Example CURLs for RAG server using the Bioma Actor framework

# Launch the server:
cargo run --release -p bioma_llm --example rag_server

# Reset the engine:
curl -X POST http://localhost:8080/reset

# Index some files:
curl -X POST http://localhost:8080/index -H "Content-Type: application/json" -d '{"globs": ["/Users/rozgo/BiomaAI/bioma/bioma_*/**/*.rs"]}'

# Retrieve context:
curl -X POST http://localhost:8080/retrieve -H "Content-Type: application/json" -d '{"query": "Can I make a game with Bioma?", "threshold": 0.0, "limit": 10}'

# Ask a question:
curl -X POST http://localhost:8080/ask -H "Content-Type: application/json" -d '{"query": "Can I make a game with Bioma?"}'


curl -X POST http://localhost:8080/api/embed -H "Content-Type: application/json" -d '{
  "model": "nomic-embed-text",
  "input": ["Why is the sky blue?", "Why is the grass green?"]
}'