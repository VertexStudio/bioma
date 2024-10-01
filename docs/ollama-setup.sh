#! /bin/bash

# Pull all models used by bioma

curl -X POST http://localhost:11434/api/pull -d '{"name":"starcoder2:3b"}'

curl -X POST http://localhost:11434/api/pull -d '{"name":"llama3.2"}'

curl -X POST http://localhost:11434/api/pull -d '{"name":"gemma2:2b"}'

curl -X POST http://localhost:11434/api/pull -d '{"name":"bespoke-minicheck"}'
