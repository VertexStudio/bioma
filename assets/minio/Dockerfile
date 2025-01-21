FROM debian:bookworm-slim

# Install required dependencies
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

# Create console directory
WORKDIR /app

# Debug: Show build context contents
RUN ls -la

# Print current directory
RUN pwd

# Copy the console binary and ensure proper permissions
COPY console /app/
RUN chmod +x /app/console

# Expose console port
EXPOSE 9090

# Run console server
ENTRYPOINT ["/app/console"]
CMD ["server"]