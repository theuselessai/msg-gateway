# Pipelit Gateway — Development Plan

**Status:** Draft
**Author:** Yao
**Date:** 2026-03-07
**Design Doc:** gateway-design.md

---

## Summary

Build a standalone, framework-agnostic Rust message gateway that bridges user-facing communication protocols (Telegram, Discord, Slack, Email, CLI) to backend agent protocols (Pipelit webhook, OpenCode, generic HTTP). Features multi-credential management, file handling, health monitoring with emergency alerting, and a Python SDK + CLI tool.

## Phase 1: Foundation (Week 1-2)

**Goal:** Minimal Rust binary with config loading, HTTP server, Telegram adapter, and webhook backend adapter.

### 1.1 Project Scaffolding

- [ ] Initialize Rust project with Cargo workspace
  - `gateway/` — main binary
  - `gateway-sdk-python/` — Python SDK (later phase)
- [ ] Set up CI: cargo build, cargo test, clippy, fmt
- [ ] Choose and configure dependencies:
  - `tokio` — async runtime
  - `axum` — HTTP server
  - `serde` / `serde_json` — config parsing
  - `notify` — fsnotify file watching
  - `tracing` — structured logging
  - `reqwest` — HTTP client (for backend calls)

### 1.2 Config System

- [ ] Define config JSON schema as Rust structs (serde)
- [ ] `route` field as opaque `Map<String, Value>` — gateway passes through without interpretation
- [ ] `target` field per credential with fallback to `gateway.default_target`
- [ ] Backend target type enum: `webhook`, `opencode`
- [ ] Implement env var resolution (`${VAR}` syntax)
- [ ] Config file loading with validation
- [ ] fsnotify watcher with debounce (avoid rapid reloads)
- [ ] Config diff logic: detect added/removed/modified credentials

### 1.3 HTTP Server

- [ ] Axum server with configurable listen address
- [ ] `POST /api/v1/send` — outbound endpoint (JSON and multipart)
- [ ] `GET /files/{file_id}` — file download endpoint
- [ ] `GET /admin/health` — basic health response
- [ ] Bearer token auth middleware for admin, send, and file scopes
- [ ] Webhook route registration (dynamic, based on config)

### 1.4 Webhook Backend Adapter

- [ ] POST normalized envelope to `inbound_url` with bearer auth
- [ ] Per-credential target resolution (credential target or default)
- [ ] Handle 202 response
- [ ] Basic retry on failure

### 1.5 Telegram User-Facing Adapter (First Protocol)

- [ ] Implement poll mode using `teloxide` long polling
- [ ] Message normalization: Telegram message → normalized envelope
- [ ] Forward normalized message to backend adapter
- [ ] Outbound send (JSON): receive on `/api/v1/send`, resolve credential, call Telegram API
- [ ] Outbound send (multipart): receive file + payload, upload to Telegram via multipart
- [ ] Graceful start/stop per credential task

### 1.6 File Handling

- [ ] Inbound: download file from Telegram (`getFile` API) at receive time
- [ ] File cache: local directory storage with unique IDs
- [ ] TTL-based cleanup with configurable interval
- [ ] Max cache size eviction (oldest first)
- [ ] File size limit: reject files exceeding `max_file_size_mb`, replace with metadata stub in envelope
- [ ] MIME type validation: allowed/blocked lists with wildcard support
- [ ] `GET /files/{file_id}` endpoint: serve cached files with auth, 404/410 on missing/expired

### 1.7 Credential Task Manager

- [ ] Task registry: HashMap<credential_id, JoinHandle>
- [ ] Spawn task on credential add / activate
- [ ] Cancel task on credential remove / deactivate
- [ ] Restart task on credential modify
- [ ] React to config file changes via watcher
- [ ] Only spawn tasks for credentials where `active: true`

**Milestone:** Gateway binary receives a Telegram message, downloads any attachments, forwards the normalized envelope to Pipelit's inbound endpoint, and delivers Pipelit's response (with optional file) back to Telegram. Credentials can be added/removed via config file edit.

---

## Phase 2: Admin API (Week 3)

**Goal:** Full CRUD on credentials via REST, without editing config files.

### 2.1 Admin Endpoints

- [ ] `GET /admin/credentials` — list all (redact tokens)
- [ ] `GET /admin/credentials/:id` — get one with runtime status
- [ ] `POST /admin/credentials` — create (validate, write config, spawn task)
- [ ] `PUT /admin/credentials/:id` — update (validate, write config, restart task)
- [ ] `DELETE /admin/credentials/:id` — delete (stop task, remove from config)
- [ ] `PATCH /admin/credentials/:id/activate` — set active, spawn task
- [ ] `PATCH /admin/credentials/:id/deactivate` — set inactive, stop task

### 2.2 Config File Write-Back

- [ ] Admin API writes changes back to config.json atomically (write temp → rename)
- [ ] Watcher ignores self-triggered file changes (avoid infinite loop)
- [ ] Concurrent access protection (single writer lock)

### 2.3 Runtime Status

- [ ] Track per-credential connection state (connecting, connected, error, stopped)
- [ ] Include runtime status in GET responses
- [ ] Enhanced `/admin/health` with per-credential status and backend reachability

**Milestone:** Full credential lifecycle via REST API. Create, activate, deactivate, delete — all via HTTP. No config file editing required.

---

## Phase 3: Emergency Mode + Health Monitoring (Week 4)

**Goal:** Gateway detects backend outages and alerts emergency contacts.

### 3.1 Health Check Loop

- [ ] Background task polling configured health endpoints at interval
- [ ] Failure counter with configurable threshold (`alert_after_failures`)
- [ ] State machine: healthy → degraded → down → recovering → healthy

### 3.2 Emergency Alerting

- [ ] On state transition to `down`: send alert to all emergency credentials
- [ ] On state transition to `healthy` (from down): send recovery notification
- [ ] Alert includes: last healthy timestamp, failure count, buffered message count

### 3.3 Message Buffering

- [ ] In-memory buffer for inbound messages when backend is unreachable
- [ ] Configurable max buffer size (drop oldest or reject new)
- [ ] Drain buffer on recovery (deliver in order)
- [ ] Buffer metrics (current size, messages dropped)

### 3.4 Inbound Retry Logic

- [ ] Exponential backoff on backend POST failure (1s, 2s, 4s, 8s, max 30s)
- [ ] Max retry count before buffering
- [ ] Structured logging for all retry/buffer events

**Milestone:** Stop Pipelit, receive Telegram alert on emergency credential. Start Pipelit, receive recovery notification and see buffered messages delivered.

---

## Phase 4: Additional User-Facing Protocols (Week 5-6)

**Goal:** Support Discord, Slack, Email, and CLI as user-facing adapters.

### 4.1 Discord Adapter

- [ ] WebSocket gateway connection using `serenity`
- [ ] Message normalization (guild, channel, DM handling)
- [ ] Outbound send via REST API
- [ ] Reconnection and session resume handling
- [ ] File download from Discord CDN at receive time

### 4.2 Slack Adapter

- [ ] Webhook mode via Events API
- [ ] URL verification challenge handling
- [ ] Message normalization (channels, DMs, threads)
- [ ] Outbound send via `chat.postMessage`
- [ ] File download with bot token auth at receive time

### 4.3 Email Adapter

- [ ] IMAP IDLE for push-like inbound (using `async-imap`)
- [ ] SMTP outbound
- [ ] Email → normalized message (subject as text, body, attachments extracted from MIME)
- [ ] Thread tracking via Message-ID / In-Reply-To headers

### 4.4 CLI Adapter

- [ ] stdin/stdout adapter for local development
- [ ] Same normalized envelope as all other protocols
- [ ] Useful for testing workflows without external services

**Milestone:** All five user-facing protocols working. Messages flow through any of them to the configured backend.

---

## Phase 5: OpenCode Backend Adapter (Week 7)

**Goal:** Support OpenCode server as a backend target.

### 5.1 OpenCode Adapter

- [ ] Session management: create/reuse sessions per credential or conversation
- [ ] Send messages via `POST /session/:id/prompt_async`
- [ ] Poll for responses via messages API or SSE event stream
- [ ] Map OpenCode response parts to normalized outbound envelope
- [ ] Drop `attachments` from inbound (OpenCode does not support files)
- [ ] Deliver response through user-facing protocol adapter
- [ ] Pass `route` fields (agent, model, etc.) to OpenCode API

### 5.2 Backend Adapter Trait

- [ ] Define common trait for backend adapters: `send_message`, `supports_files`
- [ ] Webhook adapter implements trait
- [ ] OpenCode adapter implements trait
- [ ] Credential task manager dispatches to correct adapter based on `target.protocol`

**Milestone:** A Discord credential routes to OpenCode, sends a coding question, OpenCode processes it, gateway polls for response, delivers back to Discord.

---

## Phase 6: Python SDK + CLI Tool (Week 8)

**Goal:** Python client library and command-line tool for managing the gateway.

### 6.1 Python SDK

- [ ] `pipelit-gateway` PyPI package
- [ ] `GatewayClient` class wrapping REST API
- [ ] `gw.send()` — outbound messaging (JSON and multipart with file)
- [ ] `gw.credentials.*` — CRUD operations
- [ ] `gw.health()` — health check
- [ ] Async support (httpx)
- [ ] Error handling with typed exceptions

### 6.2 CLI Tool

- [ ] `gw-cli` command using `typer`
- [ ] `credentials list|create|update|delete|activate|deactivate`
- [ ] `send` — test message delivery (text and file)
- [ ] `health` — gateway and backend status
- [ ] Config via env vars or `~/.gw-cli.json`
- [ ] Table-formatted output for list commands

### 6.3 Pipelit Integration

- [ ] Add `pipelit-gateway` as dependency to Pipelit server
- [ ] Replace direct protocol polling with SDK `gw.send()` calls
- [ ] Add `/api/v1/inbound` endpoint to Pipelit's FastAPI server
- [ ] Wire inbound messages to workflow trigger execution

**Milestone:** `gw-cli credentials list` shows all credentials. `gw-cli send` delivers a test message. Pipelit uses the SDK for all outbound messaging.

---

## Phase 7: Observability + Hardening (Week 9)

**Goal:** Production-ready monitoring, metrics, and resilience.

### 7.1 Structured Logging

- [ ] JSON-formatted logs via `tracing-subscriber`
- [ ] Log all message events with: credential_id, user protocol, backend protocol, direction, latency, status
- [ ] Log config changes and task lifecycle events

### 7.2 Metrics

- [ ] Prometheus metrics endpoint (`/metrics`)
- [ ] Message counters by protocol, credential, direction
- [ ] Latency histograms
- [ ] Health check status, active credentials, buffer size, file cache size

### 7.3 Outbound Retry

- [ ] Protocol-specific retry logic (respect Telegram 429, Discord rate limits)
- [ ] Return structured error responses to backend on permanent failure

### 7.4 TLS

- [ ] Optional TLS termination for webhook mode
- [ ] Configurable cert/key paths

### 7.5 Graceful Shutdown

- [ ] SIGTERM handler: drain in-flight messages, stop pollers, flush buffer
- [ ] Configurable shutdown timeout

**Milestone:** Grafana dashboard showing message throughput, latency, and credential health. Gateway survives protocol errors, backend outages, and restarts.

---

## Phase 8: Packaging + Release (Week 10)

**Goal:** Distributable binaries and documentation.

### 8.1 Build and Distribution

- [ ] Multi-platform builds (Linux x86_64, ARM64, macOS)
- [ ] Docker image (minimal, scratch-based)
- [ ] GitHub releases with prebuilt binaries
- [ ] Cargo feature flags per protocol (compile only what you need)

### 8.2 Documentation

- [ ] README with quickstart
- [ ] Config reference
- [ ] API reference
- [ ] Protocol-specific setup guides (Telegram bot setup, Slack app setup, etc.)
- [ ] Architecture diagram

### 8.3 Open Source Preparation

- [ ] Apache 2.0 license
- [ ] Contributing guide
- [ ] Separate repo from Pipelit core
- [ ] PyPI publish for `pipelit-gateway` SDK

**Milestone:** `cargo install pipelit-gateway` or download a binary, write a config file, and have a working multi-protocol message gateway.

---

## Dependencies and Risks

| Risk | Mitigation |
|------|-----------|
| Teloxide API changes | Pin version, abstract behind adapter trait |
| Protocol rate limits during testing | Use test bot tokens with low traffic |
| Config file corruption on concurrent writes | Atomic write (temp + rename) + single writer lock |
| Message loss during gateway restart | Buffer persistence (open question — may add WAL in later phase) |
| Scope creep into routing logic | Enforce: gateway is dumb. One credential → one backend. No matching, no routing intelligence. |
| OpenCode API changes | Pin to known version, isolate in adapter |

## Non-Goals (Explicit)

- **No routing logic in the gateway** — all message routing is handled by the backend's workflow/agent nodes.
- **No message persistence** — the gateway buffers transiently during outages but is not a message store.
- **No multi-tenancy at the gateway level** — tenant isolation is the backend's responsibility.
- **No LLM integration** — the gateway doesn't call AI models. It moves messages.

## Success Criteria

1. Add a new user (new Telegram bot) via `gw-cli credentials create` — message flows within 60 seconds.
2. Backend goes down — emergency credential receives alert within 90 seconds.
3. Backend recovers — buffered messages delivered in order, recovery notification sent.
4. Gateway restart — all credentials reconnect automatically from config file.
5. Outbound message from any backend — delivered to correct protocol and chat via correct credential.
6. Switch a credential from Pipelit to OpenCode — change target in config, messages reroute immediately.
