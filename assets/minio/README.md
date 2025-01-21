# MinIO Console Docker Image

This Docker image contains the MinIO Console, a graphical user interface for managing MinIO Object Storage servers.

## Building the Console Binary

The console binary can be built from source using the following steps:

1. Clone the repository:

```bash
git clone https://github.com/VertexStudio/console
cd console
```

2. Switch to the UI v2 branch:

```bash
git checkout feature/ui-v2
```

3. Build the binary:

```bash
go build -trimpath --tags=kqueue --ldflags "-s -w" -o console ./cmd/console
```

The resulting `console` binary can be used to replace the existing binary in this repository.

## Environment Variables

The following environment variables are available to configure the console:

- `CONSOLE_MINIO_SERVER`: MinIO Server endpoint (e.g., http://minio:9000)
- `CONSOLE_PORT`: Port to run the console on (default: "9090")
- `CONSOLE_SECURE_CONTENT_SECURITY_POLICY`: CSP configuration for the console
- `CONSOLE_BROWSER_REDIRECT_URL`: URL to redirect after authentication
- `CONSOLE_PBKDF_PASSPHRASE`: Salt to encrypt JWT payload (if using JWT authentication)
- `CONSOLE_PBKDF_SALT`: Required to encrypt JWT payload (if using JWT authentication)

## Usage

### Using Docker Run

To run the container:

```bash
docker run -d \
  -e CONSOLE_MINIO_SERVER=http://minio:9000 \
  -p 9090:9090 \
  <image-name>
```

The console will be available at `http://localhost:9090`.
