# Use NVIDIA CUDA base image
FROM nvidia/cuda:12.3.1-base-ubuntu22.04

# Prevent interactive prompts during installation
ENV DEBIAN_FRONTEND=noninteractive

# Install necessary dependencies
RUN apt-get update && apt-get install -y \
    wget \
    git \
    ca-certificates \
    curl \
    build-essential \
    nvidia-container-runtime \
    tree \
    vim

# Download and install Go
RUN wget https://go.dev/dl/go1.24.0.linux-amd64.tar.gz && \
    tar -C /usr/local -xzf go1.24.0.linux-amd64.tar.gz && \
    rm go1.24.0.linux-amd64.tar.gz

# Configure Go environment variables
ENV GOROOT=/usr/local/go
ENV PATH=$GOROOT/bin:$PATH
ENV GOPATH=/go
ENV PATH=$GOPATH/bin:$PATH

# Verify Go installation
RUN go version

# Set working directory
WORKDIR /workdir

# Clone Ollama repository
RUN git clone --branch bug/tool-prop-type https://github.com/rozgo/ollama.git .

# Change to repository directory
WORKDIR /workdir

# Build Ollama
RUN go mod tidy && \
    go mod download && \
    go build -v -o /usr/local/bin/ollama

# Create entrypoint script
RUN echo '#!/bin/bash\n\
ollama serve &\n\
sleep 5\n\
ollama pull qwq:latest\n\
wait' > /entrypoint.sh && \
    chmod +x /entrypoint.sh

# Configure GPU environment variables
ENV OLLAMA_HOST=0.0.0.0
ENV NVIDIA_VISIBLE_DEVICES=all
ENV NVIDIA_DRIVER_CAPABILITIES=compute,utility

# Expose port
EXPOSE 11434

# Use the custom entrypoint script
ENTRYPOINT ["/entrypoint.sh"]