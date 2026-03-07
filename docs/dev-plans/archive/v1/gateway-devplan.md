# Pipelit Gateway — Development Plan

**Status:** Draft
**Author:** Yao
**Date:** 2026-03-07
**Design Doc:** gateway-design.md

---

## Summary

Build a standalone, framework-agnostic Rust message gateway that handles multi-protocol message ingestion, outbound delivery, credential lifecycle, and health monitoring. Designed for Pipelit but usable by any backend via opaque route configuration. Includes a Python SDK and CLI tool.

## Phase 1: Foundation (Week 1-2)

**Goal:** Minimal Rust binary with config loading, HTTP server, and one protocol adapter.

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
  - `reqwest` — HTTP client (for Pipelit calls)

### 1.2 Config System

- [ ] Define config JSON schema as Rust structs (serde)
- [ ] `route` field as opaque `Map<String, Value>` — gateway passes through without interpretation
- [ ] Implement env var resolution (`${VAR}` syntax)
- [ ] Config file loading with validation
- [ ] fsnotify watcher with debounce (avoid rapid reloads)
- [ ] Config diff logic: detect added/removed/modified credentials

### 1.3 HTTP Server

- [ ] Axum server with configurable listen address
- [ ] `POST /api/v1/send` — outbound endpoint (accepts message, returns 200 stub)
- [ ] `GET /admin/health` — basic health response
- [ ] Bearer token auth middleware for admin and send scopes
- [ ] Webhook route registration (dynamic, based on config)

### 1.4 Telegram Adapter (First Protocol)

- [ ] Implement poll mode using `teloxide` long polling
- [ ] Message normalization: Telegram message → normalized envelope
- [ ] POST normalized message to target server inbound URL
- [ ] Outbound send (JSON): receive on `/api/v1/send`, resolve credential, call Telegram API
- [ ] Outbound send (multipart): receive file + payload, upload to Telegram via multipart
- [ ] Inbound file handling: download file from Telegram (`getFile` API), store in file cache, include gateway download URL in envelope
- [ ] `GET /files/{file_id}` endpoint: serve cached files with auth
- [ ] File cache: local directory, TTL-based cleanup, max cache size eviction
- [ ] File size limit: reject files exceeding `max_file_size_mb`, replace with metadata stub in envelope
- [ ] MIME type validation: allowed/blocked lists with wildcard support, reject dangerous file types
- [ ] Graceful start/stop per credential task

### 1.5 Credential Task Manager

- [ ] Task registry: HashMap<credential_id, JoinHandle>
- [ ] Spawn task on credential add / activate
- [ ] Cancel task on credential remove / deactivate
- [ ] Restart task on credential modify
- [ ] React to config file changes via watcher

**Milestone:** Gateway binary receives a Telegram message, forwards it to a Pipelit endpoint, and delivers Pipelit's response back to Telegram. Credentials can be added/removed via config file edit.

---

## Phase 2: Admin API + Active/Inactive (Week 3)

**Goal:** Full CRUD on credentials via REST, without editing config files.

### 2.1 Admin Endpoints

- [ ] `GET /admin/credentials` — list all (redact tokens)
- [ ] `GET /admin/credentials/:id` — get one
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
- [ ] Enhanced `/admin/health` with per-credential status

**Milestone:** Full credential lifecycle via REST API. Create a credential, see it start polling, deactivate it, reactivate it, delete it — all via HTTP.

---

## Phase 3: Emergency Mode + Health Monitoring (Week 4)

**Goal:** Gateway detects Pipelit outages and alerts emergency contacts.

### 3.1 Health Check Loop

- [ ] Background task polling Pipelit's health endpoint at configured interval
- [ ] Failure counter with configurable threshold (`alert_after_failures`)
- [ ] State machine: healthy → degraded → down → recovering → healthy

### 3.2 Emergency Alerting

- [ ] On state transition to `down`: send alert to all emergency credentials
- [ ] On state transition to `healthy` (from down): send recovery notification
- [ ] Alert message includes: last healthy timestamp, failure count, buffered message count

### 3.3 Message Buffering

- [ ] In-memory buffer for inbound messages when Pipelit is unreachable
- [ ] Configurable max buffer size (drop oldest or reject new)
- [ ] Drain buffer on recovery (deliver in order)
- [ ] Buffer metrics (current size, messages dropped)

### 3.4 Inbound Retry Logic

- [ ] Exponential backoff on Pipelit inbound POST failure (1s, 2s, 4s, 8s, max 30s)
- [ ] Max retry count before buffering
- [ ] Structured logging for all retry/buffer events

**Milestone:** Stop Pipelit, receive a Telegram alert on the emergency credential. Start Pipelit, receive recovery notification and see buffered messages delivered.

---

## Phase 4: Additional Protocols (Week 5-6)

**Goal:** Support Discord, Slack, Email, and CLI.

### 4.1 Discord Adapter

- [ ] WebSocket gateway connection using `serenity`
- [ ] Message normalization (guild, channel, DM handling)
- [ ] Outbound send via REST API
- [ ] Reconnection and session resume handling

### 4.2 Slack Adapter

- [ ] Webhook mode via Events API
- [ ] URL verification challenge handling
- [ ] Message normalization (channels, DMs, threads)
- [ ] Outbound send via `chat.postMessage`

### 4.3 Email Adapter

- [ ] IMAP IDLE for push-like inbound (using `async-imap`)
- [ ] SMTP outbound
- [ ] Email → normalized message (subject, body, attachments)
- [ ] Thread tracking via Message-ID / In-Reply-To headers

### 4.4 Generic Adapter

- [ ] `POST /api/v1/chat/{credential_id}` — REST inbound endpoint (fire and forget, returns 202)
- [ ] WebSocket `/ws/chat/{credential_id}/{chat_id}` — outbound message push
- [ ] WebSocket connection registry keyed by `(credential_id, chat_id)`
- [ ] Bearer token auth for both REST and WebSocket endpoints
- [ ] Message normalization: same envelope as all other protocols
- [ ] Client-managed `chat_id` (e.g., UUID generated by web/TUI client)
- [ ] No message buffering — if WebSocket disconnected, outbound messages are dropped with log

**Milestone:** All five protocols working. A workflow can receive from Telegram and reply via Discord if needed. A TUI or web client can chat via the generic protocol.

---

## Phase 5: Python SDK + CLI Tool (Week 7)

**Goal:** Python client library and command-line tool for managing the gateway.

### 5.1 Python SDK

- [ ] `pipelit-gateway` PyPI package
- [ ] `GatewayClient` class wrapping REST API
- [ ] `gw.send()` — outbound messaging
- [ ] `gw.credentials.*` — CRUD operations
- [ ] `gw.health()` — health check
- [ ] Async support (httpx)
- [ ] Error handling with typed exceptions
- [ ] Token auth configuration

### 5.2 CLI Tool

- [ ] `gw-cli` command using `click` or `typer`
- [ ] `credentials list|create|update|delete|activate|deactivate`
- [ ] `send` — test message delivery
- [ ] `health` — gateway and Pipelit status
- [ ] Config via env vars or `~/.gw-cli.json`
- [ ] Table-formatted output for list commands

### 5.3 Pipelit Integration

- [ ] Add `pipelit-gateway` as dependency to Pipelit server
- [ ] Replace direct protocol polling with SDK `gw.send()` calls
- [ ] Add `/api/v1/inbound` endpoint to Pipelit's FastAPI server
- [ ] Wire inbound messages to workflow trigger execution

**Milestone:** `gw-cli credentials list` shows all credentials. `gw-cli send` delivers a test message. Pipelit uses the SDK for all outbound messaging.

---

## Phase 6: Observability + Hardening (Week 8)

**Goal:** Production-ready monitoring, metrics, and resilience.

### 6.1 Structured Logging

- [ ] JSON-formatted logs via `tracing-subscriber`
- [ ] Log all message events: credential_id, protocol, direction, latency, status
- [ ] Log config changes and task lifecycle events
- [ ] Configurable log level

### 6.2 Metrics

- [ ] Prometheus metrics endpoint (`/metrics`)
- [ ] `gateway_messages_inbound_total` (protocol, credential)
- [ ] `gateway_messages_outbound_total` (protocol, credential)
- [ ] `gateway_message_latency_seconds` (direction, protocol)
- [ ] `gateway_pipelit_health_status`
- [ ] `gateway_active_credentials` (protocol)
- [ ] `gateway_buffer_size`

### 6.3 Outbound Retry

- [ ] Protocol-specific retry logic (respect Telegram 429, Discord rate limits)
- [ ] Return structured error responses to Pipelit on permanent failure

### 6.4 TLS

- [ ] Optional TLS termination for webhook mode
- [ ] Configurable cert/key paths

### 6.5 Graceful Shutdown

- [ ] SIGTERM handler: drain in-flight messages, stop pollers, flush buffer
- [ ] Configurable shutdown timeout

**Milestone:** Grafana dashboard showing message throughput, latency, and credential health. Gateway survives and recovers from protocol errors, Pipelit outages, and restarts.

---

## Phase 7: Packaging + Release (Week 9)

**Goal:** Distributable binaries and documentation.

### 7.1 Build and Distribution

- [ ] Multi-platform builds (Linux x86_64, ARM64, macOS)
- [ ] Docker image (minimal, scratch-based)
- [ ] GitHub releases with prebuilt binaries
- [ ] Cargo feature flags per protocol (compile only what you need)

### 7.2 Documentation

- [ ] README with quickstart
- [ ] Config reference
- [ ] API reference
- [ ] Protocol-specific setup guides (Telegram bot setup, Slack app setup, etc.)
- [ ] Architecture diagram

### 7.3 Open Source Preparation

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
| Scope creep into agent logic | Enforce: gateway is dumb. No matching, no routing intelligence. One credential → one route. |

## Non-Goals (Explicit)

- **No routing logic in the gateway** — all message routing is handled by Pipelit's workflow nodes.
- **No message persistence** — the gateway buffers transiently during outages but is not a message store.
- **No multi-tenancy at the gateway level** — tenant isolation is Pipelit's responsibility via workspaces and sandboxes.
- **No LLM integration** — the gateway doesn't call AI models. It moves messages.

## Success Criteria

1. Add a new user (new Telegram bot) via `gw-cli credentials create` — message flows within 60 seconds.
2. Pipelit goes down — emergency credential receives alert within 90 seconds.
3. Pipelit recovers — buffered messages delivered in order, recovery notification sent.
4. Gateway restart — all credentials reconnect automatically from config file.
5. Outbound message from any workflow — delivered to correct protocol and chat via correct credential.
