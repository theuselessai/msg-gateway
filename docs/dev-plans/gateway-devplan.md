# Pipelit Gateway — Development Plan

**Status:** Active
**Author:** Yao
**Date:** 2026-03-07
**Design Doc:** gateway-design.md
**Version:** 4

---

## Summary

Build a standalone Rust message gateway that bridges user-facing protocols (Telegram, Discord, Slack, Email, Generic) to backend protocols (Pipelit, OpenCode). User-facing adapters are external processes managed by the gateway. Features multi-credential management, file handling, health monitoring with emergency alerting, and a Python SDK + CLI tool.

## Current Progress

### Completed

- [x] Project scaffolding with Cargo
- [x] Config system: JSON structs, `${ENV_VAR}` resolution, fsnotify watcher
- [x] HTTP server (axum) with Bearer token auth middleware
- [x] Credential Task Manager: TaskRegistry, spawn/stop/sync tasks
- [x] Admin API CRUD for credentials
- [x] Emergency Mode: HealthMonitor, message buffering, alerts
- [x] Generic protocol adapter (built-in): REST + WebSocket
- [x] Backend adapter refactor: `BackendProtocol` enum, `TargetConfig`, `default_target`
- [x] `BackendAdapter` trait with `PipelitAdapter` implementation

### Architecture Change

Previous design had user-facing adapters as Rust code inside the gateway. New design:
- User-facing adapters are **external processes** (any language)
- Gateway manages adapter lifecycle (spawn, health check, stop)
- Communication via HTTP (adapter exposes `/send`, `/health`; gateway exposes `/api/v1/adapter/inbound`)
- Similar to how Claude Desktop manages MCP servers

---

## Phase 1: External Adapter Infrastructure

**Goal:** Gateway can spawn and manage external adapter processes, communicate via HTTP.

### 1.1 Config Changes

- [ ] Add `adapters_dir` to `GatewayConfig`
- [ ] Add `adapter_port_range` to `GatewayConfig`
- [ ] Change `CredentialConfig.protocol` to `CredentialConfig.adapter` (string)
- [ ] Remove `mode` field (no webhook mode for now)
- [ ] Add `config` field to `CredentialConfig` (adapter-specific config)
- [ ] Update `config.example.json`

### 1.2 Adapter Discovery

- [ ] Scan `adapters_dir` on startup
- [ ] Parse `adapter.json` from each subdirectory
- [ ] Store adapter definitions: `HashMap<String, AdapterDef>`
- [ ] Validate credentials reference valid adapters (or "generic")

### 1.3 Adapter Instance Manager

- [ ] Replace `TaskRegistry` with `AdapterInstanceManager`
- [ ] `AdapterInstance` struct: instance_id, adapter_name, credential_id, port, pid, status
- [ ] Spawn adapter process with env vars:
  - `INSTANCE_ID`
  - `ADAPTER_PORT`
  - `GATEWAY_URL`
  - `CREDENTIAL_ID`
  - `CREDENTIAL_TOKEN`
  - `CREDENTIAL_CONFIG`
- [ ] Port allocation from `adapter_port_range`
- [ ] Poll `/health` until ready (with timeout)
- [ ] Track running instances: `HashMap<String, AdapterInstance>` (by instance_id)
- [ ] Lookup by credential_id for outbound routing

### 1.4 Adapter Lifecycle

- [ ] Start adapter on credential activate
- [ ] Stop adapter (SIGTERM) on credential deactivate
- [ ] Restart adapter on credential config change
- [ ] Handle adapter crash (detect via process exit, update status)

### 1.5 Inbound Endpoint

- [ ] New endpoint: `POST /api/v1/adapter/inbound`
- [ ] Validate `instance_id` exists
- [ ] Lookup `credential_id` from instance
- [ ] Download file if present, store in cache
- [ ] Build `InboundMessage` with route from credential config
- [ ] Forward to backend adapter

### 1.6 Outbound Routing

- [ ] Update `/api/v1/send` handler
- [ ] Lookup `credential_id` → `instance_id` → `port`
- [ ] POST to `http://localhost:{port}/send`
- [ ] Handle adapter errors (return to backend)

### 1.7 Generic Adapter Exception

- [ ] Keep generic adapter built-in (no external process)
- [ ] Detect `adapter: "generic"` in credential config
- [ ] Route to existing generic.rs handlers

**Milestone:** Gateway spawns a mock adapter process, receives inbound message, forwards to backend, receives outbound, routes to adapter.

---

## Phase 2: Telegram Adapter (Python)

**Goal:** First real external adapter implementation.

### 2.1 Adapter Structure

```
adapters/telegram/
├── adapter.json
├── main.py
├── requirements.txt
└── README.md
```

### 2.2 Telegram Adapter Implementation

- [ ] HTTP server (Flask/FastAPI) with `/send` and `/health`
- [ ] Connect to Telegram Bot API (long polling or MCP)
- [ ] Message normalization → POST to gateway `/api/v1/adapter/inbound`
- [ ] `/send` handler → send to Telegram API
- [ ] File handling:
  - Inbound: extract file URL/auth, include in POST
  - Outbound: read from `file_path`, upload to Telegram
- [ ] Graceful shutdown on SIGTERM

### 2.3 Integration Test

- [ ] Gateway spawns Telegram adapter
- [ ] Send message via Telegram → arrives at mock backend
- [ ] Mock backend responds → delivered to Telegram

**Milestone:** Full Telegram integration via external adapter.

---

## Phase 3: Additional Adapters

**Goal:** Discord, Slack, Email adapters.

### 3.1 Discord Adapter

- [ ] `adapters/discord/` structure
- [ ] Connect via MCP or direct API
- [ ] Same interface: `/send`, `/health`, POST inbound

### 3.2 Slack Adapter

- [ ] `adapters/slack/` structure
- [ ] Connect via MCP
- [ ] Same interface

### 3.3 Email Adapter

- [ ] `adapters/email/` structure
- [ ] IMAP for inbound, SMTP for outbound
- [ ] Same interface

**Milestone:** All four external adapters working.

---

## Phase 4: OpenCode Backend Adapter

**Goal:** Support OpenCode as backend target.

### 4.1 OpenCode Adapter

- [ ] Session management per credential
- [ ] Send via `prompt_async`
- [ ] Poll for responses
- [ ] Drop attachments (not supported)
- [ ] Deliver response to user adapter

**Milestone:** Credential with `target.protocol: "opencode"` routes correctly.

---

## Phase 5: Python SDK + CLI

**Goal:** Python client library and CLI tool.

### 5.1 SDK

- [ ] `pipelit-gateway` PyPI package
- [ ] `GatewayClient` class
- [ ] `send()`, `credentials.*`, `health()`
- [ ] Async support

### 5.2 CLI

- [ ] `plit` command
- [ ] `credentials list|create|activate|deactivate|delete`
- [ ] `send --credential --chat --text`
- [ ] `health`

**Milestone:** `plit send` delivers a message.

---

## Phase 6: Observability + Hardening

### 6.1 Logging

- [ ] JSON-formatted logs
- [ ] Log adapter spawn/stop events
- [ ] Log message flow with latency

### 6.2 Metrics

- [ ] Prometheus `/metrics` endpoint
- [ ] Message counters, latency histograms
- [ ] Adapter status metrics

### 6.3 Adapter Recovery

- [ ] Detect adapter crash
- [ ] Auto-restart with backoff
- [ ] Max restart attempts before giving up

### 6.4 Graceful Shutdown

- [ ] SIGTERM handler
- [ ] Stop all adapters gracefully
- [ ] Drain in-flight messages

**Milestone:** Production-ready observability.

---

## Phase 7: Packaging

### 7.1 Build

- [ ] Multi-platform binaries
- [ ] Docker image with Python for adapters
- [ ] GitHub releases

### 7.2 Documentation

- [ ] README with quickstart
- [ ] Adapter development guide
- [ ] Config reference

**Milestone:** Downloadable release.

---

## Non-Goals

- **No webhook mode** (deferred) — adapters initiate connections only
- **No routing logic** — gateway is dumb, backend handles routing
- **No message persistence** — in-memory buffering only
- **No multi-tenancy** — backend's responsibility

## Success Criteria

1. Add new Telegram bot via config change → working in 60 seconds
2. Backend down → emergency notification sent
3. Backend recovers → buffered messages delivered
4. Gateway restart → adapters auto-reconnect
5. Switch credential from Pipelit to OpenCode → immediate reroute
