# msg-gateway

<p align="center">
  <strong>Multi-protocol message gateway for LLM agents</strong>
</p>

<p align="center">
  <a href="https://github.com/theuselessai/msg-gateway/actions/workflows/ci.yml"><img src="https://github.com/theuselessai/msg-gateway/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="https://app.codecov.io/gh/theuselessai/msg-gateway"><img alt="Codecov" src="https://img.shields.io/codecov/c/github/theuselessai/msg-gateway?style=flat-square"></a>
  <a href="https://github.com/theuselessai/msg-gateway/releases"><img src="https://img.shields.io/github/v/tag/theuselessai/msg-gateway?label=version&style=flat-square" alt="Version" /></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-Apache%202.0-blue.svg?style=flat-square" alt="License: Apache 2.0" /></a>
</p>

---

A standalone Rust message gateway that bridges user-facing communication protocols (Telegram, Discord, Slack, Email) to backend agent protocols (Pipelit, OpenCode). External adapters are subprocess-managed, making it easy to add new protocols in any language.

## Features

- **Multi-protocol support** — Telegram, Discord, Slack, Email, Generic HTTP/WebSocket
- **External adapter architecture** — Adapters run as separate processes, written in any language
- **Backend agnostic** — Supports Pipelit (webhook) and OpenCode (REST+SSE) backends
- **File handling** — Automatic download/upload of attachments with local caching
- **Health monitoring** — Emergency alerts when backend is unreachable
- **Hot reload** — Config changes apply without restart
- **Admin API** — CRUD operations for credentials

## Quick Start

```bash
# Build
cargo build --release

# Configure
cp config.example.json config.json
# Edit config.json with your credentials

# Run
GATEWAY_CONFIG=config.json ./target/release/msg-gateway
```

## Configuration

```json
{
  "gateway": {
    "listen": "0.0.0.0:8080",
    "admin_token": "your-admin-token",
    "default_target": {
      "protocol": "pipelit",
      "inbound_url": "http://localhost:5000/api/v1/inbound",
      "token": "your-backend-token"
    },
    "adapters_dir": "./adapters",
    "adapter_port_range": [9000, 9100]
  },
  "auth": {
    "send_token": "your-send-token"
  },
  "credentials": {
    "my_telegram": {
      "adapter": "telegram",
      "token": "${TELEGRAM_BOT_TOKEN}",
      "active": true,
      "route": {"channel": "telegram"}
    }
  }
}
```

Environment variables can be referenced with `${VAR_NAME}` syntax.

## Adapters

Adapters are external processes in `adapters_dir`. Each adapter directory contains:

```
adapters/telegram/
├── adapter.json    # {"name": "telegram", "command": "python3", "args": ["main.py"]}
├── main.py         # Adapter implementation
└── requirements.txt
```

Adapters receive environment variables:
- `INSTANCE_ID` — Unique instance identifier
- `ADAPTER_PORT` — Port to listen on
- `GATEWAY_URL` — Gateway callback URL
- `CREDENTIAL_ID` — Credential identifier
- `CREDENTIAL_TOKEN` — Protocol auth token
- `CREDENTIAL_CONFIG` — JSON config blob

### Built-in Generic Adapter

The `generic` adapter is built-in and provides REST + WebSocket interface:

```bash
# Send message via REST
curl -X POST http://localhost:8080/api/v1/chat/my_generic \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"text": "Hello"}'

# Connect via WebSocket
wscat -c ws://localhost:8080/ws/chat/my_generic/session1 \
  -H "Authorization: Bearer $TOKEN"
```

## API Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /health` | Health check |
| `POST /api/v1/send` | Send message to user (backend → gateway → adapter) |
| `POST /api/v1/adapter/inbound` | Receive message from adapter |
| `GET /files/{id}` | Download cached file |
| `GET /admin/credentials` | List credentials |
| `POST /admin/credentials` | Create credential |
| `PATCH /admin/credentials/{id}/activate` | Activate credential |

## Development

```bash
# Run tests
cargo test

# Run with coverage
cargo llvm-cov

# Lint
cargo clippy -- -D warnings

# Format
cargo fmt
```

## License

Apache 2.0
