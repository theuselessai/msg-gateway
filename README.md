# plit-gw

<p align="center">
  <strong>Multi-protocol message gateway for LLM agents</strong>
</p>

<p align="center">
  <a href="https://crates.io/crates/plit"><img src="https://img.shields.io/crates/v/plit.svg?style=flat-square" alt="crates.io" /></a>
  <a href="https://github.com/theuselessai/plit-gw/actions/workflows/ci.yml"><img src="https://github.com/theuselessai/plit-gw/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="https://app.codecov.io/gh/theuselessai/plit-gw"><img alt="Codecov" src="https://img.shields.io/codecov/c/github/theuselessai/plit-gw?style=flat-square"></a>
  <a href="https://github.com/theuselessai/plit-gw/releases"><img src="https://img.shields.io/github/v/tag/theuselessai/plit-gw?label=version&style=flat-square" alt="Version" /></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-Apache%202.0-blue.svg?style=flat-square" alt="License: Apache 2.0" /></a>
</p>

---

A standalone Rust message gateway that bridges user-facing communication protocols (Telegram, Discord, Slack, Email) to backend agent protocols (Pipelit, OpenCode). Both user-facing adapters and backends can run as external subprocesses, making it easy to add new protocols in any language.

## Features

- **Multi-protocol support** — Telegram, Discord, Slack, Email, Generic HTTP/WebSocket
- **External adapter architecture** — Adapters run as separate processes, written in any language
- **Pluggable backends** — Built-in support for Pipelit and OpenCode; external backends run as subprocesses
- **Named backend routing** — Route different credentials to different backend instances
- **Message filtering** — CEL-based guardrails for inbound/outbound message validation
- **File handling** — Automatic download/upload of attachments with local caching
- **Health monitoring** — Emergency alerts when backend is unreachable
- **Hot reload** — Config and guardrail changes apply without restart
- **Admin API** — CRUD operations for credentials
- **`plit` CLI tool** — Pipelit ecosystem CLI for chat, admin, and agent integration

## Quick Start

```bash
# Install
cargo install plit

# Bootstrap (interactive — sets up Pipelit, LLM provider, credentials)
plit init

# Launch the full stack
plit start

# Chat with your AI agent
plit chat default_agent --chat-id my-session
```

### Manual Setup (advanced)

```bash
# Build from source
cargo build --release

# Configure
cp config.example.json config.json
# Edit config.json with your credentials

# Run gateway only
GATEWAY_CONFIG=config.json ./target/release/plit-gw
```

## Configuration

```json
{
  "gateway": {
    "listen": "0.0.0.0:8080",
    "admin_token": "${GATEWAY_ADMIN_TOKEN}",
    "default_backend": "pipelit",
    "adapters_dir": "./adapters",
    "adapter_port_range": [9000, 9100],
    "guardrails_dir": "./guardrails",
    "backends_dir": "./backends",
    "backend_port_range": [9200, 9300]
  },
  "backends": {
    "pipelit": {
      "protocol": "pipelit",
      "inbound_url": "http://localhost:8000/api/v1/inbound",
      "token": "${PIPELIT_API_TOKEN}",
      "active": true
    },
    "opencode": {
      "protocol": "external",
      "adapter_dir": "./backends/opencode",
      "token": "${OPENCODE_BACKEND_TOKEN}",
      "active": true,
      "config": {
        "base_url": "http://127.0.0.1:4096",
        "token": "${OPENCODE_API_TOKEN}",
        "model": {
          "providerID": "anthropic",
          "modelID": "claude-sonnet-4-5"
        }
      }
    }
  },
  "auth": {
    "send_token": "${GATEWAY_SEND_TOKEN}"
  },
  "credentials": {
    "my_telegram": {
      "adapter": "telegram",
      "backend": "pipelit",
      "token": "${TELEGRAM_BOT_TOKEN}",
      "active": true,
      "route": {"channel": "telegram"}
    }
  }
}
```

Environment variables can be referenced with `${VAR_NAME}` syntax.

## Backends

Backends receive messages from the gateway and process them with AI/LLM services. The gateway supports two backend types:

### Built-in Backends

Built-in backends are compiled into the gateway binary:

- **Pipelit** (`protocol: "pipelit"`) — Webhook-based backend with callback support
- **OpenCode** (`protocol: "opencode"`) — REST + SSE backend with session management (built-in Rust implementation)

### External Backends

External backends run as separate subprocesses in `backends_dir`. Each backend directory contains:

```
backends/opencode/
├── adapter.json    # {"name": "opencode", "command": "node", "args": ["dist/main.js"]}
├── dist/main.js    # Backend implementation
└── package.json
```

External backends receive environment variables:
- `BACKEND_PORT` — Port to listen on
- `GATEWAY_URL` — Gateway callback URL
- `BACKEND_TOKEN` — Auth token for gateway requests
- `BACKEND_CONFIG` — JSON config blob (from `config.backends[name].config`)

External backends must implement:
- `POST /send` — Receive messages from gateway
- `GET /health` — Health check endpoint

Each credential specifies which backend to route to via the `backend` field. The gateway spawns one backend instance per named backend entry in `config.backends`, shared across all credentials referencing that backend.

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
size(message.attachments) > 0

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
  "expression": "size(message.attachments) > 0",
  "action": "log"
}
```

### Limitations

- **`matches()` uses Rust regex syntax**, not RE2 or the Google CEL spec. Lookaheads and backreferences are not supported. Case-insensitive matching uses the `(?i)` flag.
- **`has()` is not available.** `Option<T>` fields serialize as `null` when `None`, so CEL sees them as `null` rather than absent. However, fields with `skip_serializing_if` (like `attachments` when empty) are omitted from the CEL context entirely. Use `on_error: "allow"` (the default) so rules referencing omitted fields fail open instead of blocking.
- Outbound guardrails are not evaluated in v1. Only `"direction": "inbound"` rules take effect.

### Hot reload

Guardrail rules reload automatically when rule files in `guardrails_dir` change. No restart needed.

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

## CLI Tool (`plit`)

A standalone command-line client for interacting with the gateway. Supports interactive chat, one-shot messaging, WebSocket streaming, credential management, and health checks. Backend-agnostic — works with Pipelit, OpenCode, or any external backend.

### Install

```bash
cargo build --release -p plit
# Binary at target/release/plit
```

### Usage

```bash
# Set connection defaults
export GATEWAY_URL=http://localhost:8080
export GATEWAY_TOKEN=my-credential-token

# Interactive chat REPL
plit chat my_credential --chat-id session-1

# One-shot send (pipe-friendly)
plit send my_credential --chat-id session-1 --text "Hello"
echo "Hello" | plit send my_credential --chat-id session-1

# Stream responses as JSONL (for agents, scripts, jq)
plit listen my_credential --chat-id session-1

# Health check
plit health

# Credential management (requires GATEWAY_ADMIN_TOKEN)
plit credentials list --admin-token my-admin-token
plit credentials create my_cred --adapter generic --token secret \
  --backend pipelit --route '{"workflow_slug":"my-wf","trigger_node_id":"node_1"}'
plit credentials activate my_cred
plit credentials deactivate my_cred
```

### Output Modes

- **TTY** (interactive terminal) — human-readable formatted output
- **Piped** (stdout redirected) — auto-switches to JSON/JSONL
- **`--json`** flag — force JSON output in any context

### Environment Variables

| Variable | Used by | Description |
|----------|---------|-------------|
| `GATEWAY_URL` | all commands | Gateway URL (default: `http://localhost:8080`) |
| `GATEWAY_TOKEN` | chat, send, listen | Credential token for authentication |
| `GATEWAY_ADMIN_TOKEN` | credentials, health | Admin token for management commands |

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

### Local Setup

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

### Pre-Push Validation (Recommended)

Install a pre-push hook to catch issues before CI:

```bash
cat > .git/hooks/pre-push << 'EOF'
#!/bin/bash
set -e
echo "Running pre-push checks..."
cargo fmt --all -- --check || { echo "❌ Run: cargo fmt --all"; exit 1; }
cargo clippy --all-targets --all-features -- -D warnings || { echo "❌ Fix clippy warnings"; exit 1; }
cargo test --all-features || { echo "❌ Tests failed"; exit 1; }
echo "✅ All checks passed"
EOF
chmod +x .git/hooks/pre-push
```

This prevents formatting, linting, and test failures from reaching CI.

## Contributing

Contributions are welcome! Please read [CLAUDE.md](CLAUDE.md) for detailed development guidelines.

### Quick Start

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Install the pre-push hook (see Development section above)
4. Make your changes
5. Run the full check suite: `cargo fmt --all && cargo clippy -- -D warnings && cargo test`
6. Commit your changes (`git commit -m 'feat: add amazing feature'`)
7. Push to the branch (`git push origin feature/amazing-feature`)
8. Open a Pull Request

### PR Quality Checklist

Before opening a PR, verify:

- [ ] `cargo fmt --all` — Code is formatted
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` — No warnings
- [ ] `cargo test --all-features` — All tests pass
- [ ] `cargo build --release` — Release build succeeds
- [ ] Error handling uses `Result<?>`/`map_err` (no `unwrap()`/`expect()` in production code)
- [ ] Structured logging includes relevant context fields (e.g., `credential_id`, `message_id`)
- [ ] Config secrets use `${ENV_VAR}` syntax (no hardcoded tokens)
- [ ] New functionality includes tests

All PRs must pass CI checks (lint, test, build) and AI code review.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.
