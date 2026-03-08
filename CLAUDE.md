# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with this repository.

## Project Overview

**msg-gateway** is a standalone Rust message gateway that bridges user-facing communication protocols (Telegram, Discord, Slack, Email, Generic HTTP/WS) to backend agent protocols (Pipelit, OpenCode).

## Architecture

```
User Protocols          Gateway              Backend Protocols
┌──────────────┐      ┌─────────┐           ┌─────────────┐
│   Telegram   │─────▶│         │──────────▶│   Pipelit   │
│   Discord    │─────▶│   msg   │──────────▶│  (webhook)  │
│    Slack     │─────▶│ gateway │           └─────────────┘
│    Email     │─────▶│         │           ┌─────────────┐
│   Generic    │─────▶│         │──────────▶│  OpenCode   │
└──────────────┘      └─────────┘           │ (REST+SSE)  │
                                            └─────────────┘
```

- **External adapters**: User-facing adapters run as subprocesses managed by the gateway
- **Generic adapter**: Built-in, no external process (REST + WebSocket)
- **Config hot reload**: File watcher detects changes and syncs adapter instances
- **Health monitoring**: Buffers messages when backend is down, sends alerts

## Key Files

| File | Purpose |
|------|---------|
| `src/main.rs` | Entry point |
| `src/server.rs` | HTTP routes (Axum) |
| `src/config.rs` | Config structs, env var resolution |
| `src/adapter.rs` | External adapter process management |
| `src/generic.rs` | Built-in generic adapter (REST + WebSocket) |
| `src/manager.rs` | Credential task registry |
| `src/watcher.rs` | Config hot reload |
| `src/health.rs` | Health monitoring, emergency mode |
| `src/backend.rs` | Backend protocol adapters (Pipelit, OpenCode) |
| `src/files.rs` | File cache for attachments |
| `src/admin.rs` | Admin API CRUD |
| `src/error.rs` | Error types |

## Common Commands

```bash
# Build
cargo build --release

# Run tests
cargo test

# Run with coverage
cargo llvm-cov

# Lint
cargo clippy -- -D warnings

# Format
cargo fmt

# Run the gateway
GATEWAY_CONFIG=config.json cargo run
```

## Development Guidelines

1. **Branch Protection**: Never push directly to master. Create feature branch and submit PR.
2. **CI Checks**: All PRs must pass lint, test, and build checks.
3. **AI Code Review**: PRs are reviewed by gito.bot with Anthropic Claude.
4. **Test Coverage**: Target 80%+ coverage. Use `cargo llvm-cov` to check.
5. **Config Secrets**: Use `${ENV_VAR}` syntax in config for sensitive values.
6. **PR Merging**: NEVER auto-merge PRs (no `gh pr merge --admin` or any merge command). Always create the PR, report the URL, and wait for a human to merge.

## Project Workflow

Full workflow definition: [`docs/workflow.md`](docs/workflow.md)

**Issue statuses:** Backlog → Ready → In Review → Done

**Atlas must:**
- Move issue to **In Review** when a PR is opened (`Closes #N` in PR body)
- Move issue to **Done** when a PR is merged
- After merge: check roadmap for newly unblocked issues → move to **Ready** if dev plan exists, else leave in **Backlog**
- Before marking **Ready**: confirm all blockers are Done AND a dev plan exists at `docs/dev-plans/{issue-slug}.md` (or equivalent documentation is sufficient)
- Remove the `blocked` label from issues when their blockers are resolved
- PRs must include `Closes #N` in the body to link the issue

## Config Structure

```json
{
  "gateway": {
    "listen": "0.0.0.0:8080",
    "admin_token": "${ADMIN_TOKEN}",
    "default_target": {
      "protocol": "pipelit",
      "inbound_url": "http://backend/inbound",
      "token": "${BACKEND_TOKEN}"
    }
  },
  "auth": {
    "send_token": "${SEND_TOKEN}"
  },
  "credentials": {
    "my_cred": {
      "adapter": "telegram",
      "token": "${TELEGRAM_TOKEN}",
      "active": true,
      "route": {}
    }
  }
}
```

## Testing Notes

- Tests modifying `GATEWAY_CONFIG` env var need `#[serial]` from `serial_test` crate
- `std::env::set_var` requires `unsafe` block in Rust 2024
- Integration tests are in `tests/integration_test.rs`
- WebSocket test (`tests/ws_test.rs`) requires running server

## External Adapter Protocol

Adapters receive these environment variables:
- `INSTANCE_ID`: Unique instance ID
- `ADAPTER_PORT`: Port to listen on
- `GATEWAY_URL`: Gateway callback URL
- `CREDENTIAL_TOKEN`: Auth token for the protocol

Adapters must implement:
- `GET /health`: Return `{"status": "ok"}`
- `POST /send`: Send message to user, return `{"protocol_message_id": "..."}`
- POST to `${GATEWAY_URL}/api/v1/adapter/inbound` for inbound messages
