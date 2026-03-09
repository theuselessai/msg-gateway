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

## Guardrails

Guardrails let you filter inbound messages using [CEL (Common Expression Language)](https://cel.dev) expressions. Each rule is a JSON file in `guardrails_dir`. Rules are evaluated in lexicographic filename order, so zero-padded prefixes (`01-`, `02-`, ...) give you predictable ordering.

### Rule format

Each file contains a single JSON object:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | string | required | Human-readable rule name |
| `type` | `"cel"` | `"cel"` | Rule type (only CEL supported) |
| `expression` | string | required | CEL expression that must evaluate to `bool` |
| `action` | `"block"` or `"log"` | `"block"` | What to do when the expression is true |
| `direction` | `"inbound"`, `"outbound"`, or `"both"` | `"inbound"` | Which messages to apply the rule to |
| `on_error` | `"allow"` or `"block"` | `"allow"` | Behavior when CEL evaluation fails |
| `reject_message` | string | none | Body returned in the HTTP 403 response when blocked |
| `enabled` | bool | `true` | Set to `false` to disable without deleting the file |

### CEL expression examples

The `message` variable is available in every expression:

```
# Block messages containing sensitive keywords (Rust regex syntax)
message.text.matches("(?i)(password|secret|api_key)")

# Block messages over 10000 characters
size(message.text) > 10000

# Log messages that include file attachments (never blocks)
size(message.files) > 0

# Block messages from a specific source protocol
message.source.protocol == "telegram"
```

### Example rule files

```json
{
  "name": "block-sensitive-keywords",
  "expression": "message.text.matches(\"(?i)(password|secret|api_key)\")",
  "action": "block",
  "reject_message": "Message contains sensitive keywords and cannot be forwarded."
}
```

```json
{
  "name": "audit-attachments",
  "expression": "size(message.files) > 0",
  "action": "log"
}
```

### Limitations

- **`matches()` uses Rust regex syntax**, not RE2 or the Google CEL spec. Lookaheads and backreferences are not supported. Case-insensitive matching uses the `(?i)` flag.
- **`has()` is not available.** Fields that may be absent (like `files`) are pre-converted to `null` by the gateway. Use `on_error: "allow"` (the default) to handle missing fields gracefully instead of failing closed.
- Outbound guardrails are not evaluated in v1. Only `"direction": "inbound"` rules take effect.

### Hot reload

Guardrail rules reload automatically when the config file changes. No restart needed. New or modified rule files in `guardrails_dir` are picked up on the next reload cycle.

### Configuration

Point `guardrails_dir` at a directory of rule files:

```json
{
  "gateway": {
    "guardrails_dir": "./guardrails"
  }
}
```

If `guardrails_dir` is omitted and a `guardrails/` directory exists next to `config.json`, it's picked up automatically.

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

## Contributing

Contributions are welcome! Please read [CLAUDE.md](CLAUDE.md) for development guidelines.

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

All PRs must pass CI checks (lint, test, build) and AI code review.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.
