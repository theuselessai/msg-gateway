# Issue #33 — Backend Adapters as External Scripting Processes

## 2026-03-09 — Initial context

### Goal
Allow backend adapters (currently hardcoded Rust: Pipelit, OpenCode) to run as external subprocess-managed processes, similar to how protocol adapters (Telegram, Discord) work. This enables backend logic to be written in TypeScript/Python/etc.

### Key Design Decisions
- New `BackendProtocol::External` variant in `config.rs`
- New `ExternalBackendAdapter` in `backend.rs` — spawns subprocess, calls `POST /send`, receives response via gateway self-relay
- Backend subprocess receives env vars: `GATEWAY_URL`, `BACKEND_TOKEN`, `BACKEND_CONFIG`
- Must implement: `GET /health` → `{"status":"ok"}`, `POST /send` → processes message, relays response via `POST {GATEWAY_URL}/api/v1/send`
- Config: `target.protocol = "external"`, `target.adapter_dir = "./backends/my-backend"`
- Lifecycle management: health checks, restarts, backoff — reuse patterns from `adapter.rs`

### Existing Patterns to Follow
- `adapter.rs`: `AdapterInstanceManager`, `PortAllocator`, `AdapterProcess`, `AdapterDef` — reuse these patterns
- `adapter.rs` spawn: sets env vars `INSTANCE_ID`, `ADAPTER_PORT`, `GATEWAY_URL`, `CREDENTIAL_ID`, `CREDENTIAL_TOKEN`, `CREDENTIAL_CONFIG`
- Backend adapters get: `GATEWAY_URL`, `BACKEND_TOKEN`, `BACKEND_CONFIG` (analogous)
- `adapters/telegram/adapter.json`: `{"name":"telegram","version":"2.0.0","command":"node","args":["dist/main.js"]}`
- `BackendAdapter` trait: `async fn send_message(&self, msg: &InboundMessage) -> Result<(), BackendError>`
- `create_adapter()` factory in `backend.rs` — add `BackendProtocol::External` arm

### Architecture
The `ExternalBackendAdapter` in Rust:
1. On first use: spawn subprocess from `adapter_dir`, allocate port, wait for health
2. `POST /send` to subprocess with the `InboundMessage` as JSON body + `Authorization: Bearer {token}`
3. Subprocess processes message, calls `POST {GATEWAY_URL}/api/v1/send` to relay response back
4. Rust adapter returns `Ok(())` after successful POST to subprocess

### Example TypeScript Backend Adapter
Create `backends/opencode/` as the reference implementation (TypeScript, Node.js):
- `adapter.json`: `{"name":"opencode","version":"1.0.0","command":"node","args":["dist/main.js"]}`
- Receives `InboundMessage` via `POST /send`
- Calls OpenCode server API
- Relays response via `POST {GATEWAY_URL}/api/v1/send`

### Codebase Conventions
- Edition 2024 — `std::env::set_var` requires `unsafe` block
- `tracing` only for logging (structured: `tracing::info!(field = %val, "msg")`)
- `Arc<RwLock<T>>` for shared state
- `#[serde(default)]` for optional fields
- `thiserror` for domain errors
- `async_trait` for trait methods
- No `println!/eprintln!`
- `-D warnings` enforced — no dead code, no unused imports

## 2026-03-09 — Implementation Complete

### Files Changed
- `src/config.rs`: Added `BackendProtocol::External`, `adapter_dir`/`port` to `TargetConfig`, `backends_dir`/`backend_port_range` to `GatewayConfig`
- `src/backend.rs`: `ExternalBackendAdapter` (POST to subprocess), `ExternalBackendManager` (lifecycle), `create_adapter()` External arm
- `src/admin.rs`, `src/manager.rs`, `src/watcher.rs`, `tests/integration_test.rs`: Updated TargetConfig/GatewayConfig constructions
- `backends/opencode/`: TypeScript reference adapter (Fastify, session management, OpenCode API)
- `.gitignore`: Added backends/opencode/ ignore patterns

### Gotchas Encountered
- `Arc<dyn BackendAdapter>` doesn't implement `Debug`, so `unwrap_err()` fails on `Result<Arc<dyn BackendAdapter>, _>` — use `match` instead
- `TargetConfig` and `GatewayConfig` are constructed directly in 6+ locations across tests — ALL need updating when adding fields
- `#[allow(dead_code)]` needed on `ExternalBackendManager` since it's not yet wired into `main.rs`
- Clippy catches `map(|s| PathBuf::from(s))` → must use `map(PathBuf::from)` (redundant closure)
