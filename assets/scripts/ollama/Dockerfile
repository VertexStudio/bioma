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
    nvidia-container-runtime

# Descargar y extraer Go
RUN wget https://github.com/golang/go/archive/refs/tags/go1.24.0.tar.gz && \
    tar -xzf go1.24.0.tar.gz && \
    mv go-go1.24.0 /usr/local/go

# Configurar variables de entorno para Go
ENV GOROOT=/usr/local/go
ENV PATH=$GOROOT/bin:$PATH

# Establecer directorio de trabajo
WORKDIR /workdir

# Clonar el repositorio de Ollama
RUN git clone --branch bug/tool-prop-type https://github.com/rozgo/ollama.git .

# Cambiar al directorio del repositorio clonado
WORKDIR /workdir/ollama

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