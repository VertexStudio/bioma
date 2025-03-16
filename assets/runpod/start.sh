#!/bin/bash

# RunPod startup script for Ollama with CUDA support
# This script runs every time the pod starts and:
# 1. Sets up the environment
# 2. Builds Ollama with CUDA support
# 3. Starts the Ollama service
# 4. Loads the default model
#
# Assumes:
# - RunPod template with CUDA support
# - Ollama source code in /workspace/ollama
# - Go installed in /workspace/go
# - Environment configuration in /workspace/bashrc

# Source environment variables directly from bashrc
. /workspace/bashrc

# Also set up for SSH sessions
cat /workspace/bashrc >> /root/.bashrc

# Install dependencies
apt update && apt install -y cmake nano

# Build Ollama with CUDA support
cd /workspace/ollama
cmake -B build
cmake --build build

# Start Ollama service
echo "Starting Ollama..."
ollama serve > /workspace/ollama.log 2>&1
