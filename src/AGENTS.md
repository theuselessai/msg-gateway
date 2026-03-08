# src/ — Core Gateway Modules

14 flat Rust modules. No subdirectories. All re-exported via `lib.rs` for integration tests.

## MODULE MAP

| Module | Lines | Role | Risk |
|--------|-------|------|------|
| `adapter.rs` | ~1720 | External adapter subprocess lifecycle (spawn, health, restart) | HIGH — largest file, complex state |
| `server.rs` | ~947 | Axum HTTP server, routes, AppState, middleware | HIGH — central hub |
| `files.rs` | ~1035 | File cache (download, store, serve, TTL, MIME validation) | MED |
| `manager.rs` | ~802 | CredentialManager + TaskRegistry for instance tracking | MED |
| `config.rs` | ~482 | Config structs, env var `${VAR}` resolution, defaults | MED — 8 dependents |
| `admin.rs` | ~380 | Admin CRUD endpoints, atomic config writes | LOW — isolated |
| `health.rs` | ~381 | HealthMonitor state machine, message buffering, alerts | LOW — isolated |
| `generic.rs` | ~350 | Built-in generic adapter (REST + WS), WsRegistry | LOW — isolated |
| `watcher.rs` | ~239 | Config hot-reload via `notify` crate, adapter sync | LOW |
| `error.rs` | ~236 | `AppError` enum (thiserror), `IntoResponse` for Axum | LOW — but 8 dependents |
| `backend.rs` | ~210 | `BackendAdapter` trait, PipelitAdapter, OpencodeAdapter (stub) | LOW |
| `message.rs` | ~75 | InboundMessage, OutboundMessage, Attachment, UserInfo | LOW — pure data |
| `main.rs` | ~135 | Entry point: init tracing, load config, spawn tasks | LOW |
| `lib.rs` | ~16 | Module re-exports for integration tests | TRIVIAL |

## KEY TYPES

- **`AppState`** (server.rs) — Shared state: `config: RwLock<Config>`, `ws_registry`, `manager`, `adapter_manager`, `health_monitor`, `file_cache`
- **`BackendAdapter`** (backend.rs) — `async fn send_message(&self, msg: &InboundMessage) -> Result<(), BackendError>`
- **`InboundMessage`** (message.rs) — Normalized envelope: route, credential_id, source (protocol, chat_id, from), text, attachments, timestamp
- **`AppError`** (error.rs) — Enum variants map to HTTP status codes; implements `IntoResponse`
- **`AdapterInstanceManager`** (adapter.rs) — Manages subprocess lifecycle; keyed by credential_id
- **`HealthMonitor`** (health.rs) — State machine + message buffer (max 1000)
- **`WsRegistry`** (generic.rs) — `Arc<RwLock<HashMap<(cred_id, chat_id), broadcast::Sender>>>`

## PATTERNS TO FOLLOW

- **New handler**: Add route in `server.rs` → extract `State<Arc<AppState>>` → return `Result<impl IntoResponse, AppError>`
- **New backend**: Implement `BackendAdapter` trait in `backend.rs` → add variant to `BackendProtocol` enum in `config.rs`
- **New error variant**: Add to `AppError` enum → add `IntoResponse` match arm with HTTP status
- **Shared state access**: `state.config.read().await` for reads, `state.config.write().await` for writes
- **Background task**: `tokio::spawn()` from `main.rs` → pass `Arc<AppState>` clone

## GOTCHAS

- `adapter.rs` uses `OnceLock<reqwest::Client>` for static HTTP client (lazy init)
- `watcher.rs` checks `skip_reload_until` before processing file events (admin API coordination)
- `admin.rs` writes config atomically (temp file → rename) and sets skip_reload_until
- `generic.rs` inbound is fire-and-forget: spawns async task, returns 202 immediately
- `health.rs` emergency alerts are TODO — currently log-only, no HTTP POST

