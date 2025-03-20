FROM ubuntu:24.04 AS builder

# Instalar dependencias necesarias
RUN apt-get update && apt-get install -y curl tar && rm -rf /var/lib/apt/lists/*

# Descargar y extraer SurrealDB 2.2.1
RUN curl -L -o surreal.tgz https://github.com/surrealdb/surrealdb/releases/download/v2.2.1/surreal-v2.2.1.linux-amd64.tgz && \
    tar -xzf surreal.tgz && \
    mv surreal /usr/local/bin/surreal && \
    chmod +x /usr/local/bin/surreal && \
    rm surreal.tgz

# Imagen final
FROM ubuntu:24.04

# Copiar el binario de SurrealDB desde la imagen builder
COPY --from=builder /usr/local/bin/surreal /usr/local/bin/surreal

# Crear directorio para la base de datos
#RUN mkdir -p /data

# Exponer el puerto de SurrealDB
EXPOSE 9123

# Comando por defecto para iniciar SurrealDB
CMD ["surreal", "start", "--no-banner", "--allow-all", "--bind", "0.0.0.0:9123", "--user", "root", "--pass", "root", "surrealkv:/workspace/bioma.db"]
