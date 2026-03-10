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
| `src/guardrail.rs` | CEL-based message filtering engine |
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
3. **AI Code Review**: PRs are reviewed by gito.bot with Anthropic Claude Sonnet 4.5.
4. **Test Coverage**: Target 80%+ coverage. Use `cargo llvm-cov` to check.
5. **Config Secrets**: Use `${ENV_VAR}` syntax in config for sensitive values.
6. **PR Merging**: NEVER auto-merge PRs (no `gh pr merge --admin` or any merge command). Always create the PR, report the URL, and wait for a human to merge.

## Code Quality & Review Preparation

### Pre-Push Validation (MANDATORY)

**Before pushing**, run the full CI check suite locally:

```bash
# One-liner to match CI pipeline
cargo fmt --all && \
cargo clippy --all-targets --all-features -- -D warnings && \
cargo test --all-features && \
cargo build --release
```

**Install pre-push hook** to enforce automatically:

```bash
cat > .git/hooks/pre-push << 'EOF'
#!/bin/bash
set -e
echo "🔍 Running pre-push validation..."
cargo fmt --all -- --check || { echo "❌ Formatting failed. Run: cargo fmt --all"; exit 1; }
cargo clippy --all-targets --all-features -- -D warnings || { echo "❌ Clippy warnings found. Fix above."; exit 1; }
cargo test --all-features || { echo "❌ Tests failed."; exit 1; }
echo "✅ All checks passed. Pushing..."
EOF
chmod +x .git/hooks/pre-push
```

This prevents 60%+ of review issues from ever reaching CI.

### Common gito.bot Review Patterns

Based on PR history, gito.bot frequently flags these issues:

| Category | Common Issues | Prevention |
|----------|---------------|------------|
| **Error Handling** | `unwrap()`/`expect()` in production, missing error context, improper serialization | Use `?` operator and `map_err` with context. Log errors with `tracing::error!(error = ?e, "context")` |
| **Security** | Timing attacks in auth, empty token bypass, hardcoded secrets | Use `constant_time_eq` for token comparison. Validate non-empty tokens. Use `${ENV_VAR}` in config. |
| **Concurrency** | RwLock held across `.await`, shutdown race conditions | Release locks before async operations. Check shutdown flags before long operations. |
| **Logging** | Missing context fields, inconsistent patterns | Include `credential_id`, `message_id`, `backend_name` in structured logs. |
| **Documentation** | Field name drift, unclear limitations | Update docs in same commit as code changes. Document CEL/regex limitations. |
| **Testing** | Missing coverage, validation gaps | Add unit tests for new functions, integration tests for new endpoints. |

### Pre-Submission Checklist

Before opening a PR, verify ALL items:

#### Automated Checks
- [ ] `cargo fmt --all` — Formatting clean
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` — No warnings
- [ ] `cargo test --all-features` — All tests pass (including new tests for changes)
- [ ] `cargo build --release` — Release build succeeds
- [ ] `cargo llvm-cov --all-features` — Coverage ≥80% (check HTML report with `--html`)

#### Code Quality
- [ ] **Error Handling**: No `unwrap()`/`expect()` in production code; use `?` or proper `Result` handling
- [ ] **Error Context**: All errors logged with structured fields (e.g., `credential_id`, `message_id`)
- [ ] **Security**: Auth code uses constant-time comparison; no hardcoded tokens; validate non-empty credentials
- [ ] **Concurrency**: Lock scopes minimized; no locks held across `.await` or long operations
- [ ] **Shutdown**: Long operations check shutdown flags to avoid delaying graceful shutdown

#### Logging & Observability
- [ ] **Structured Logging**: Use `tracing::info!(field = %value, "message")` not `println!`
- [ ] **Context Fields**: Include relevant IDs (`credential_id`, `backend_name`, `adapter_name`, `message_id`)
- [ ] **Error Traces**: Log error context: `tracing::error!(error = ?e, context = "operation", "failed")`

#### Documentation
- [ ] **Field Names**: Code and docs use consistent terminology (e.g., `attachments` not `files`)
- [ ] **Limitations**: Document regex syntax, CEL limitations, config constraints
- [ ] **Examples**: Config examples use `${ENV_VAR}` not placeholder tokens like `"your-token-here"`

#### Testing
- [ ] **Unit Tests**: New functions have unit tests in same file
- [ ] **Integration Tests**: New endpoints have tests in `tests/integration_test.rs`
- [ ] **Edge Cases**: Test error paths, empty inputs, invalid configs
- [ ] **Coverage**: New code is covered (check with `cargo llvm-cov --html`)

#### Configuration
- [ ] **Secrets**: All sensitive values use `${ENV_VAR}` syntax
- [ ] **Validation**: Config loading validates required fields and rejects invalid values
- [ ] **Backward Compat**: Config changes preserve existing config file compatibility (or document migration)

### Review Response Protocol

When gito.bot provides feedback:

1. **Read ALL feedback** before making changes (avoid addressing issues one-by-one across multiple commits)
2. **Group related fixes** into logical commits (e.g., "fix: address security review issues" not "fix: issue #1, #2, #3")
3. **Test after each fix** to avoid cascading failures
4. **Update PR description** with summary of changes if scope expands significantly
5. **Request re-review** after pushing fixes (gito.bot auto-reviews on push)

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
    "default_backend": "pipelit",
    "adapters_dir": "./adapters",
    "backends_dir": "./backends",
    "guardrails_dir": "./guardrails"
  },
  "backends": {
    "pipelit": {
      "protocol": "pipelit",
      "inbound_url": "http://backend/inbound",
      "token": "${BACKEND_TOKEN}",
      "active": true
    }
  },
  "auth": {
    "send_token": "${SEND_TOKEN}"
  },
  "credentials": {
    "my_cred": {
      "adapter": "telegram",
      "backend": "pipelit",
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
- `CREDENTIAL_CONFIG`: JSON config blob

Adapters must implement:
- `GET /health`: Return `{"status": "ok"}`
- `POST /send`: Send message to user, return `{"protocol_message_id": "..."}`
- POST to `${GATEWAY_URL}/api/v1/adapter/inbound` for inbound messages

## External Backend Protocol

External backends (e.g., OpenCode adapter in Node.js) run as subprocesses in `backends_dir`. They receive:
- `BACKEND_PORT`: Port to listen on
- `GATEWAY_URL`: Gateway callback URL
- `BACKEND_TOKEN`: Auth token for gateway requests
- `BACKEND_CONFIG`: JSON config blob from `config.backends[name].config`

Backends must implement:
- `POST /send`: Receive messages from gateway
- `GET /health`: Health check endpoint

## Guardrails

The gateway includes a CEL-based guardrail system for message filtering. Guardrail rules are JSON files in `guardrails_dir` that are evaluated against inbound messages. Rules support:
- CEL expressions with `message` variable
- `block` or `log` actions
- Custom rejection messages
- Hot-reload on file changes

See README.md for detailed guardrail documentation.
