# Use NVIDIA CUDA base image
FROM nvidia/cuda:12.3.1-base-ubuntu22.04

# Evitar interacciones interactivas durante la instalación
ENV DEBIAN_FRONTEND=noninteractive

# Instalar dependencias necesarias
RUN apt-get update && apt-get install -y \
    wget \
    git \
    ca-certificates \
    curl \
    build-essential \
    golang \
    nvidia-container-runtime

# Establecer directorio de trabajo
WORKDIR /app

# Clonar el repositorio de Ollama
RUN git clone --branch bug/tool-prop-type https://github.com/rozgo/ollama.git .

# Construir Ollama
RUN go mod tidy && \
    go mod download && \
    go build -o /usr/local/bin/ollama

# Descargar modelo
RUN /usr/local/bin/ollama pull qwq:latest

# Configurar variables de entorno para GPU
ENV OLLAMA_HOST=0.0.0.0
ENV NVIDIA_VISIBLE_DEVICES=all
ENV NVIDIA_DRIVER_CAPABILITIES=compute,utility

# Exponer puerto
EXPOSE 11434

# Punto de entrada
ENTRYPOINT ["/usr/local/bin/ollama", "serve"]