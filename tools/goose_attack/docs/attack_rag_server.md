# Rag Server - Stress Testing Guide

This guide details how to use the `bioma_llm` stress testing tool to evaluate the performance of your RAG server under various load conditions.

## Overview

The stress testing tool simulates multiple users interacting with different endpoints of your RAG server. It allows you to:

- Define which endpoints to test
- Set the number of concurrent users
- Control test duration
- Specify endpoint execution order
- Introduce variations in test data

## Prerequisites

RAG server must be running before starting the stress test. Please refer to the main [README](../../../README.md) file.

```bash
cargo run -p goose_attack --bin attack_rag_server -- [OPTIONS]
```

## Usage

Basic command structure:

```bash
cargo run -p goose_attack --bin attack_rag_server -- [OPTIONS]
```

### Options

| Option               | Required | Description                                                                       | Example                                   | Default                 |
| -------------------- | -------- | --------------------------------------------------------------------------------- | ----------------------------------------- | ----------------------- |
| `--endpoints`        | No       | Comma-separated list of endpoints to test with optional weights (endpoint:weight) | `--endpoints health:1,index:3,upload,ask` | `all`                   |
| `--server`           | No       | RAG server URL                                                                    | `--server http://localhost:5766`          | `http://localhost:5766` |
| `--users`            | No       | Number of concurrent users                                                        | `--users 20`                              | `10`                    |
| `--time`             | No       | Test duration in seconds                                                          | `--time 70`                               | `60`                    |
| `--log`              | No       | Request logs file path                                                            | `--log .output/requests.log`              | `.output/requests.log`  |
| `--report`           | No       | HTML report file path                                                             | `--report .output/report.html`            | `.output/report.html`   |
| `--metrics-interval` | No       | Metrics reporting interval (seconds)                                              | `--metrics-interval 4`                    | `0`                     |
| `--order`            | No       | Execute endpoints in listed order                                                 | `--order`                                 | Random order            |
| `--variations`       | No       | Number of test file variations                                                    | `--variations 16`                         | `5`                     |

### Available Endpoints

- `health`
- `hello`
- `index`
- `chat`
- `upload`
- `delete`
- `embed`
- `ask`
- `retrieve`
- `rerank`
- `all` (tests all endpoints and should be used alone)

## Example Scenarios

### Basic Test (All Endpoints)

```bash
cargo run -p goose_attack --bin attack_rag_server
```

Tests all endpoints with 10 users for 60 seconds.

### Ordered Test with Specific Endpoints

```bash
cargo run -p goose_attack --bin attack_rag_server -- --endpoints health,hello,upload --order
```

Tests specified endpoints in sequence.

### Weighted Test

```bash
cargo run -p goose_attack --bin attack_rag_server -- --endpoints index:3,ask:1
```

Tests index endpoint 3x more frequently than ask endpoint.

### Extended Test with High Load

```bash
cargo run -p goose_attack --bin attack_rag_server -- --endpoints all --users 50 --time 120 --variations 50
```

Simulates 50 users across all endpoints for 2 minutes using 50 different test files.

## Test File Variations

The `--variations` option introduces variability in test data for file-handling endpoints.

Example:

```bash
cargo run -p goose_attack --bin attack_rag_server -- --endpoints upload,index --variations 3
```

## Interpreting Results

The stress testing results can be analyzed through two main outputs:

1. HTML Report (`report.html`).

2. Request Logs (`requests.log`).

## Notes

- Ensure RAG server is running at specified URL before testing
