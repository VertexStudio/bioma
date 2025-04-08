FROM nvcr.io/nvidia/cuda:12.6.1-cudnn-devel-ubuntu22.04 AS builder

ARG ONNXRUNTIME_VERSION=1.20.0
ENV DEBIAN_FRONTEND=noninteractive

# Instalar paquetes necesarios
RUN apt-get update && apt-get install -y --no-install-recommends \
    wget \
    libssl-dev \
    pkg-config \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Crear usuario no-root
RUN useradd -m -u 1000 bioma

# Cambiar al usuario no-root
USER bioma
WORKDIR /home/bioma

# Instalar Rust
RUN curl https://sh.rustup.rs -sSf | sh -s -- -y
ENV PATH="/home/bioma/.cargo/bin:${PATH}"

# Descargar e instalar ONNXRuntime
RUN wget https://github.com/microsoft/onnxruntime/releases/download/v${ONNXRUNTIME_VERSION}/onnxruntime-linux-x64-gpu-${ONNXRUNTIME_VERSION}.tgz \
    && tar -xzf onnxruntime-linux-x64-gpu-${ONNXRUNTIME_VERSION}.tgz \
    && mv onnxruntime-linux-x64-gpu-${ONNXRUNTIME_VERSION} /home/bioma/onnxruntime \
    && rm onnxruntime-linux-x64-gpu-${ONNXRUNTIME_VERSION}.tgz

# Configurar el directorio de trabajo para la aplicación
WORKDIR /app

# Copy the entire application directory
COPY --chown=bioma:bioma . .

# Configurar variables de entorno para CUDA y ONNXRuntime
ENV PATH="/home/bioma/onnxruntime/bin:/usr/local/nvidia/bin:/usr/local/cuda/bin:${PATH}"
ENV LD_LIBRARY_PATH="/home/bioma/onnxruntime/lib:/usr/local/cuda/lib64:/usr/lib/x86_64-linux-gnu:${LD_LIBRARY_PATH}"

# Compilar la aplicación
RUN cargo build --release -p cognition --bin cognition-server

# Segunda etapa: imagen final más ligera
FROM nvcr.io/nvidia/cuda:12.6.1-cudnn-runtime-ubuntu22.04

ARG ONNXRUNTIME_VERSION=1.20.0
ENV DEBIAN_FRONTEND=noninteractive

# Instalar solo las dependencias necesarias para la ejecución
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Crear usuario no-root
RUN useradd -m -u 1000 bioma

# Crear directorio de datos
RUN mkdir -p /data && chown bioma:bioma /data

# Cambiar al usuario no-root
USER bioma
WORKDIR /home/bioma

# Copiar ONNXRuntime del builder
COPY --from=builder --chown=bioma:bioma /home/bioma/onnxruntime /home/bioma/onnxruntime

# Copiar los directorios necesarios
COPY --from=builder --chown=bioma:bioma /app/assets /app/assets
COPY --from=builder --chown=bioma:bioma /app/apps/cognition/templates /app/apps/cognition/templates
COPY --from=builder --chown=bioma:bioma /app/apps/cognition/docs /app/apps/cognition/docs

# Configurar el directorio de trabajo para la aplicación
WORKDIR /app

# Copiar solo los archivos necesarios para la ejecución
COPY --from=builder --chown=bioma:bioma /app/target/release/cognition-server /app/
COPY --chown=bioma:bioma assets/configs/rag_config_server.json /app/assets/configs/

# Configurar variables de entorno para CUDA y ONNXRuntime
ENV PATH="/home/bioma/onnxruntime/bin:/usr/local/nvidia/bin:/usr/local/cuda/bin:${PATH}"
ENV LD_LIBRARY_PATH="/home/bioma/onnxruntime/lib:/usr/local/cuda/lib64:/usr/lib/x86_64-linux-gnu:${LD_LIBRARY_PATH}"

# Exponer el puerto necesario
EXPOSE 7123

# Exponer el directorio de datos como volumen
VOLUME ["/data"]

# Configurar el nivel de log
ENV RUST_LOG=info

# Comando para iniciar la aplicación
ENTRYPOINT ["/app/cognition-server"]
CMD ["/app/assets/configs/rag_config_server.json"]