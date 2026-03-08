# PROJECT KNOWLEDGE BASE

**Generated:** 2026-03-08 **Commit:** 62a8579 **Branch:** master

## OVERVIEW

Multi-protocol Rust message gateway bridging user protocols (Telegram, Discord, Slack, Email, Generic HTTP/WS) to backend agent protocols (Pipelit, OpenCode). External adapters run as managed subprocesses; generic adapter is built-in.

## STRUCTURE

```
msg-gateway/
├── src/                    # All Rust modules (flat, 14 files, see src/AGENTS.md)
├── tests/                  # Integration + WS tests (see tests/AGENTS.md)
├── adapters/telegram/      # Example external adapter (Python)
├── docs/                   # Architecture docs, dev plans, API specs
├── .github/workflows/      # CI (lint+test+build) + AI code review
├── config.example.json     # Reference config
└── CLAUDE.md               # Dev guidelines (complementary to this file)
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Add new backend protocol | `src/backend.rs` | Implement `BackendAdapter` trait |
| Add external adapter | `adapters/{name}/` | Create `adapter.json` + impl; see Adapter Protocol below |
| Modify message shape | `src/message.rs` | Ripples to server, backend, generic, health |
| Change config schema | `src/config.rs` | Ripples to ALL modules (8 dependents) |
| Add API endpoint | `src/server.rs` | Axum routes; admin routes in `src/admin.rs` |
| Fix error handling | `src/error.rs` | `AppError` enum with HTTP status mapping (8 dependents) |
| Debug health/buffering | `src/health.rs` | State machine: Healthy→Degraded→Down→Recovering |
| Debug adapter spawning | `src/adapter.rs` | 1700+ lines; port alloc, health checks, restart backoff |
| Debug config hot-reload | `src/watcher.rs` | File watcher + `skip_reload_until` coordination with admin |
| Debug WebSocket | `src/generic.rs` | WsRegistry broadcast channels |
| File upload/download | `src/files.rs` | FileCache with TTL, MIME validation |
| Credential lifecycle | `src/manager.rs` | TaskRegistry; spawn/stop/sync operations |

## MODULE DEPENDENCY GRAPH

```
                    config.rs ←──── (8 modules depend on this)
                    error.rs  ←──── (8 modules depend on this)
                        ↑
main.rs ─→ adapter.rs ─→ manager.rs
   │           ↑              ↑
   └─→ server.rs ─→ generic.rs
           │    │─→ backend.rs ─→ message.rs
           │    │─→ health.rs
           │    │─→ admin.rs
           │    └─→ files.rs
           │
      watcher.rs (background; reads adapter, config, manager, server)
```

**Most central (change = wide ripple):** config.rs, error.rs, message.rs, server.rs (AppState)
**Most isolated (safe to modify):** files.rs, admin.rs, generic.rs, health.rs

## CONVENTIONS

- **Edition 2024** — `std::env::set_var` requires `unsafe` block
- **Error handling** — `thiserror` for domain (`AppError`), `anyhow` for main; return `Result<T, AppError>` from handlers
- **Logging** — `tracing` only. Structured: `tracing::info!(credential_id = %id, "msg")`. No println/eprintln
- **Async** — Tokio full; `#[tokio::test]` for tests; `async_trait` for trait methods
- **Concurrency** — `Arc<RwLock<T>>` for shared state; broadcast channels for WS
- **Serialization** — Serde derive on all public structs; `#[serde(default)]` for optional fields
- **Config secrets** — `${ENV_VAR}` syntax; never hardcode tokens
- **HTTP** — Axum 0.8; `State<Arc<AppState>>` extraction; reqwest with rustls-tls (no OpenSSL)
- **No custom clippy/rustfmt config** — defaults enforced with `-D warnings`

## ANTI-PATTERNS (THIS PROJECT)

- **NEVER** push directly to master — feature branch + PR required
- **NEVER** use `std::env::set_var` without `unsafe {}` block (Rust 2024)
- **NEVER** omit `#[serial]` on tests that modify env vars — causes race conditions
- **DO NOT** use deprecated `file: Option<AdapterFileInfo>` — use `files: Vec<>` (v0.2+)
- **DO NOT** use deprecated `file_path: Option<String>` — use `file_paths: Vec<>` (v0.2+)
- **BEWARE** `get_download_url()` returns URL even for non-existent files (may 404 downstream)
- **BEWARE** generic adapter health always returns true (no real health check)

## INCOMPLETE IMPLEMENTATIONS (TODOs)

| Area | File:Line | Status |
|------|-----------|--------|
| OpenCode backend adapter | `backend.rs:91-131` | Stub; returns error |
| External adapter process spawning | `manager.rs:184-205` | Placeholder; port alloc + SIGTERM missing |
| Emergency alert delivery | `health.rs:279,313` | Logs only; no HTTP POST to adapter |

## ADAPTER PROTOCOL (External)

Adapters receive env vars: `INSTANCE_ID`, `ADAPTER_PORT`, `GATEWAY_URL`, `CREDENTIAL_ID`, `CREDENTIAL_TOKEN`, `CREDENTIAL_CONFIG`

Must implement:
- `GET /health` → `{"status": "ok"}`
- `POST /send` → `{"protocol_message_id": "..."}`
- POST inbound to `${GATEWAY_URL}/api/v1/adapter/inbound`

## NOTES

- Config hot-reload + admin API coordinate via `skip_reload_until` flag (2s window) to prevent race
- Adapter health monitor: 30s interval, 3 failures → restart, exponential backoff up to 60s, max 5 restarts
- Health state machine buffers up to 1000 messages during backend outage; drains on recovery
- WS broadcast channels capacity: 100 per chat
- File cache: MIME validation, TTL expiration, metadata persistence, background cleanup
- CI: `cargo fmt` → `cargo clippy -D warnings` → `cargo llvm-cov` (80%+ target) → `cargo build --release` → gito.bot AI review
