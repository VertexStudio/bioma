FROM nvcr.io/nvidia/cuda:12.6.1-cudnn-devel-ubuntu22.04

ARG ONNXRUNTIME_VERSION=1.19.0
ENV DEBIAN_FRONTEND=noninteractive
ENV APP_NAME=bioma-service

# Install necessary packages
RUN apt-get update && apt-get install -y --no-install-recommends \
    wget \
    libssl-dev \
    pkg-config \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create a non-root user with UID 1000 to match the docker-compose configuration
RUN useradd -m -u 1000 bioma

# Switch to non-root user
USER bioma
WORKDIR /home/bioma

# Install Rust for the non-root user
RUN curl https://sh.rustup.rs -sSf | sh -s -- -y
ENV PATH="/home/bioma/.cargo/bin:${PATH}"

# Download and install ONNXRuntime for the non-root user
RUN wget https://github.com/microsoft/onnxruntime/releases/download/v${ONNXRUNTIME_VERSION}/onnxruntime-linux-x64-gpu-${ONNXRUNTIME_VERSION}.tgz \
    && tar -xzf onnxruntime-linux-x64-gpu-${ONNXRUNTIME_VERSION}.tgz \
    && mv onnxruntime-linux-x64-gpu-${ONNXRUNTIME_VERSION} /home/bioma/onnxruntime \
    && rm onnxruntime-linux-x64-gpu-${ONNXRUNTIME_VERSION}.tgz

# Set the working directory for the application
WORKDIR /app

# Copy the entire application directory
COPY --chown=bioma:bioma . .

# Set CUDA and ONNXRuntime environment variables
ENV PATH="/home/bioma/onnxruntime/bin:/usr/local/nvidia/bin:/usr/local/cuda/bin:${PATH}"
ENV LD_LIBRARY_PATH="/home/bioma/onnxruntime/lib:/usr/local/cuda/lib64:/usr/lib/x86_64-linux-gnu:${LD_LIBRARY_PATH}"

# Build the application
RUN cargo build --release

# Ensure the binary is in the correct location
RUN cp target/release/bioma-service /app/

# Create and set permissions for the /data directory
USER root
RUN mkdir -p /data && chown bioma:bioma /data
USER bioma

# Expose the necessary port
EXPOSE 7123

# Expose the target directory as a volume
VOLUME ["/data"]

# Set the startup command to run your application
CMD ["./bioma-service", "arg1", "arg2"]
