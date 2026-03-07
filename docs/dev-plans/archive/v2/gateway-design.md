# Pipelit Gateway — Design Document

**Status:** Draft
**Author:** Yao
**Date:** 2026-03-07

---

## 1. Overview

Pipelit Gateway is a standalone Rust binary that serves as a multi-protocol message bridge. It connects user-facing communication protocols (Telegram, Discord, Slack, Email, CLI) to backend agent protocols (Pipelit, OpenCode, or any webhook-compatible server).

The gateway is a protocol bridge — adapters on both sides, normalized messages in the middle. User-facing adapters handle how messages arrive from humans. Backend-facing adapters handle how messages reach agent servers and how responses come back. Adding a new messaging platform or a new backend is the same pattern: write an adapter.

The gateway is framework-agnostic. Routing metadata (`route`) is an opaque key-value map passed through without interpretation. The gateway and backend servers are fully decoupled — either can be restarted, redeployed, or scaled independently.

## 2. Problem Statement

Pipelit's current architecture tightly couples protocol polling to individual workers, with each worker bound to a single credential. This creates three pain points:

1. **Multiple credential polling** — adding a new user (e.g., a family member) requires structural changes rather than configuration changes.
2. **Multiple protocol support** — each protocol (Telegram, Slack, Discord, Email) has different connection models (long-polling, WebSocket, IMAP, webhooks) that don't fit cleanly into a single worker pattern.
3. **Outbound routing** — workflow responses need to reach the correct protocol and chat via the correct credential, requiring credential context to flow through the entire system.

## 3. Architecture

### 3.1 Protocol Bridge Model

The gateway bridges user-facing protocols to backend-facing protocols. Each credential connects one user-facing adapter to one backend adapter.

```
User-facing adapters              Backend-facing adapters
  ├── telegram (poll/webhook)       ├── webhook (Pipelit, generic)
  ├── discord  (websocket)          └── opencode (REST + SSE poll)
  ├── slack    (webhook)
  ├── email    (IMAP)
  └── cli      (stdin/stdout)
                    │                         │
                    ▼                         ▼
              ┌───────────────────────────────────┐
              │         Gateway (Rust)             │
              │                                    │
              │  ┌────────────┐  ┌──────────────┐  │
              │  │ Config     │  │ HTTP Server   │  │
              │  │ Watcher    │  │               │  │
              │  │ (fsnotify) │  │ /api/v1/send  │  │
              │  │            │  │ /files/{id}   │  │
              │  │            │  │ /webhook/...  │  │
              │  │            │  │ /admin/...    │  │
              │  └────────────┘  └──────────────┘  │
              │                                    │
              │  ┌────────────┐  ┌──────────────┐  │
              │  │ File Cache │  │ Health       │  │
              │  │ (temp dir) │  │ Monitor      │  │
              │  └────────────┘  └──────────────┘  │
              └───────────────────────────────────┘
```

### 3.2 Design Principles

- **Protocol bridge** — the gateway bridges user protocols to backend protocols. Adding either side is the same pattern: write an adapter.
- **Dumb routing** — one credential maps to one backend target. All routing intelligence lives in the backend's workflow/agent logic (e.g., Pipelit's categorizer, router, switch nodes).
- **Config-driven** — credentials, targets, and routes are defined in a JSON config file. Changes are detected via filesystem watching and applied without restart.
- **Decoupled** — gateway and backends communicate via their respective adapter protocols. No shared Redis, no shared filesystem.
- **One task per credential** — each credential spawns an independent async task (tokio). Isolation is the default; scaling is adding credentials.

## 4. Configuration

### 4.1 Config File Format

The gateway reads from a single JSON config file, watched via fsnotify for live changes. Secret values can reference environment variables using `${ENV_VAR}` syntax.

```json
{
  "gateway": {
    "listen": "0.0.0.0:8080",
    "admin_token": "${GATEWAY_ADMIN_TOKEN}",
    "default_target": {
      "protocol": "webhook",
      "inbound_url": "https://pipelit.local/api/v1/inbound",
      "token": "${PIPELIT_API_TOKEN}"
    },
    "file_cache": {
      "directory": "/var/lib/gateway/files",
      "ttl_hours": 24,
      "max_cache_size_mb": 500,
      "cleanup_interval_minutes": 30,
      "max_file_size_mb": 50,
      "allowed_mime_types": [
        "text/plain",
        "text/markdown",
        "text/csv",
        "application/pdf",
        "application/json"
      ],
      "blocked_mime_types": [
        "application/x-executable",
        "application/x-msdownload",
        "application/x-shellscript"
      ]
    }
  },
  "auth": {
    "send_token": "${GATEWAY_SEND_TOKEN}"
  },
  "health_checks": {
    "pipelit": {
      "url": "https://pipelit.local/health",
      "interval_seconds": 30,
      "alert_after_failures": 3,
      "notify_credentials": ["yao_telegram"]
    }
  },
  "credentials": {
    "yao_telegram": {
      "protocol": "telegram",
      "mode": "poll",
      "token": "${YAO_TELEGRAM_TOKEN}",
      "active": true,
      "emergency": true,
      "route": {
        "workflow_id": "wf_abc123",
        "trigger_id": "trigger_telegram_yao"
      }
    },
    "holly_telegram": {
      "protocol": "telegram",
      "mode": "webhook",
      "webhook_path": "/webhook/telegram/holly",
      "token": "${HOLLY_TELEGRAM_TOKEN}",
      "active": true,
      "emergency": false,
      "route": {
        "workflow_id": "wf_def456",
        "trigger_id": "trigger_telegram_holly"
      }
    },
    "dev_discord": {
      "protocol": "discord",
      "mode": "poll",
      "token": "${DEV_DISCORD_TOKEN}",
      "active": true,
      "emergency": false,
      "target": {
        "protocol": "opencode",
        "base_url": "https://opencode.local:4096",
        "token": "${OPENCODE_TOKEN}",
        "poll_interval_ms": 500
      },
      "route": {
        "agent": "coder"
      }
    }
  }
}
```

Credentials without a `target` field use `gateway.default_target`. Credentials with a `target` override it, enabling multi-backend routing — different credentials can route to different servers via different backend protocols.

The `route` object is opaque to the gateway. For Pipelit: `workflow_id` + `trigger_id`. For OpenCode: `agent`. For other frameworks: any custom schema.

### 4.2 Environment Variable Resolution

If a config value starts with `${` and ends with `}`, the gateway resolves it from the process environment at startup and on config reload. Missing env vars cause a startup error for required fields.

### 4.3 Credential Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `protocol` | string | yes | User-facing protocol: `telegram`, `slack`, `discord`, `email`, `cli` |
| `mode` | string | yes | Connection mode: `poll` or `webhook` |
| `token` | string | yes | Protocol auth token (or env var reference) |
| `active` | bool | yes | Whether this credential is currently enabled |
| `emergency` | bool | yes | Whether to notify via this credential when a health check fails |
| `webhook_path` | string | webhook mode | URL path the gateway registers for inbound webhooks |
| `target` | object | no | Per-credential backend target override. If omitted, uses `gateway.default_target`. See §4.4 for target types. |
| `route` | object | yes | Opaque key-value map passed through to the backend. The gateway does not interpret route contents — the receiving server defines the schema. |

### 4.4 Backend Target Types

#### Webhook (default)

Fire-and-forget POST. The backend receives the message and calls the gateway back when it has a response.

```json
{
  "protocol": "webhook",
  "inbound_url": "https://pipelit.local/api/v1/inbound",
  "token": "${PIPELIT_API_TOKEN}"
}
```

Used by: Pipelit, or any server that accepts a POST and calls back via `POST /api/v1/send`.

#### OpenCode

Session-based interaction. The gateway manages sessions, sends messages via `prompt_async`, and polls for responses via the messages API or SSE events.

```json
{
  "protocol": "opencode",
  "base_url": "https://opencode.local:4096",
  "token": "${OPENCODE_TOKEN}",
  "poll_interval_ms": 500
}
```

OpenCode does not support file attachments. The OpenCode adapter drops the `attachments` field from inbound messages and sends text only. Responses are text only.

### 4.5 Auth Scopes

Three authentication scopes protect the gateway's endpoints:

| Scope | Token Config Key | Protects |
|-------|-----------------|----------|
| Admin | `gateway.admin_token` | `GET/POST/PUT/DELETE /admin/*` |
| Send | `auth.send_token` | `POST /api/v1/send` and `GET /files/{file_id}` (backend → gateway) |
| Inbound | `default_target.token` or per-credential `target.token` | Outbound calls from gateway to backend |

All tokens are passed as `Authorization: Bearer <token>` headers. The backend auth token is resolved per-credential — each credential can authenticate to a different backend server.

## 5. API Specification

### 5.1 Inbound Message (Gateway → Backend)

**Webhook target:** the gateway POSTs normalized messages to the backend's inbound URL.

```
POST {resolved_inbound_url}
Authorization: Bearer {resolved_token}
Content-Type: application/json
```

```json
{
  "route": {
    "workflow_id": "wf_abc123",
    "trigger_id": "trigger_telegram_yao"
  },
  "credential_id": "yao_telegram",
  "source": {
    "protocol": "telegram",
    "chat_id": "123456",
    "message_id": "789",
    "from": {
      "id": "user_001",
      "username": "yao",
      "display_name": "Yao"
    }
  },
  "text": "Check the latest PR status",
  "attachments": [
    {
      "type": "image",
      "filename": "screenshot.png",
      "mime_type": "image/png",
      "size_bytes": 204800,
      "download_url": "https://gateway.local:8080/files/f_a1b2c3d4"
    }
  ],
  "timestamp": "2026-03-07T10:30:00Z"
}
```

**Response:** `202 Accepted` — the backend acknowledges receipt. The `route` field is passed through as-is from the credential config.

**OpenCode target:** the gateway creates/reuses a session and sends via `prompt_async`. The `attachments` field is dropped — OpenCode does not support file receiving. The gateway polls for the response and delivers it back through the user-facing protocol.

### 5.2 Outbound Message (Backend → Gateway)

Backends POST outbound messages to the gateway's send endpoint. Messages without attachments use JSON. Messages with attachments use multipart form data.

**Without attachments:**

```
POST https://gateway.local:8080/api/v1/send
Authorization: Bearer {auth.send_token}
Content-Type: application/json
```

```json
{
  "credential_id": "yao_telegram",
  "chat_id": "123456",
  "reply_to_message_id": "789",
  "text": "PR #102 merged. Coverage at 93%."
}
```

**With attachments:**

```
POST https://gateway.local:8080/api/v1/send
Authorization: Bearer {auth.send_token}
Content-Type: multipart/form-data; boundary=boundary

--boundary
Content-Disposition: form-data; name="payload"
Content-Type: application/json

{"credential_id": "yao_telegram", "chat_id": "123456", "reply_to_message_id": "789", "text": "Here's the report"}
--boundary
Content-Disposition: form-data; name="file"; filename="report.pdf"
Content-Type: application/pdf

<binary data>
--boundary--
```

**Response:**

```json
{
  "status": "sent",
  "protocol_message_id": "telegram_msg_456",
  "timestamp": "2026-03-07T10:30:05Z"
}
```

The gateway resolves `credential_id` to the correct user-facing protocol adapter and auth token, then delivers the message.

Note: the `/api/v1/send` endpoint is only used by webhook-type backends. OpenCode responses are handled internally by the OpenCode adapter — it polls for responses and delivers them directly through the user-facing protocol.

### 5.3 File Download (Backend → Gateway)

Backends fetch inbound attachments from the gateway's file endpoint.

```
GET /files/{file_id}
Authorization: Bearer {auth.send_token}
```

**Response:** Binary file stream with appropriate `Content-Type` and `Content-Disposition` headers.

**404** if the file ID is unknown. **410 Gone** if the file has expired (TTL exceeded).

### 5.4 Admin Endpoints

#### List Credentials

```
GET /admin/credentials
Authorization: Bearer {gateway.admin_token}
```

Returns all credentials with their status (active/inactive, connection state). Token values are redacted.

#### Get Credential

```
GET /admin/credentials/:id
Authorization: Bearer {gateway.admin_token}
```

Returns a single credential's config and runtime status.

#### Create Credential

```
POST /admin/credentials
Authorization: Bearer {gateway.admin_token}
Content-Type: application/json
```

Creates a new credential, writes to config.json, and spawns the adapter task.

#### Update Credential

```
PUT /admin/credentials/:id
Authorization: Bearer {gateway.admin_token}
Content-Type: application/json
```

Updates a credential's config. Restarts the adapter task if protocol/mode/token/target changed.

#### Activate / Deactivate

```
PATCH /admin/credentials/:id/activate
PATCH /admin/credentials/:id/deactivate
Authorization: Bearer {gateway.admin_token}
```

Quick toggle for the `active` flag. Spawns or kills the adapter task accordingly.

#### Delete Credential

```
DELETE /admin/credentials/:id
Authorization: Bearer {gateway.admin_token}
```

Gracefully stops the adapter task, removes from config.json.

#### Health

```
GET /admin/health
Authorization: Bearer {gateway.admin_token}
```

Returns gateway status: uptime, active credentials, connection states, backend reachability.

## 6. User-Facing Protocol Adapters

Each user-facing protocol adapter is an independent async task (tokio) that handles protocol-specific connections and message normalization.

### 6.1 Adapter Lifecycle

```
Config change detected
  → Diff credential set against running tasks
  → New credentials: validate → spawn task
  → Removed credentials: send cancel signal → await graceful shutdown
  → Modified credentials: cancel old → spawn new
  → active: false → cancel task (if running)
  → active: true → spawn task (if not running)
```

### 6.2 Connection Modes

| Mode | Behavior |
|------|----------|
| `poll` | Adapter spawns an async loop that polls the protocol API at a configured interval. |
| `webhook` | Adapter registers an HTTP route on the gateway's server at `webhook_path`. |

### 6.3 Supported Protocols

| Protocol | Poll | Webhook | Rust Crate | Notes |
|----------|------|---------|------------|-------|
| Telegram | Long polling | Bot API webhooks | `teloxide` | One connection per bot token |
| Slack | — | Events API | `slack-morphism` | Webhook-only; requires URL verification |
| Discord | WebSocket gateway | Interactions endpoint | `serenity` | WebSocket is the standard model |
| Email | IMAP IDLE | — | `async-imap` | Poll via IMAP IDLE (push-like) |
| CLI | stdin | — | `tokio::io` | Local development/debug interface |

### 6.4 Message Normalization

All user-facing adapters produce the same normalized message envelope regardless of protocol. Protocol-specific fields (e.g., Telegram's `chat_type`, Discord's `guild_id`) are discarded — the backend only sees the normalized format.

## 7. File Handling

Each protocol handles files differently: Telegram uses `file_id`, Slack requires auth to download, Discord CDN URLs expire, email embeds files as MIME parts. The gateway normalizes this by downloading at receive time.

### 7.1 Inbound (Protocol → Backend)

When a message arrives with attachments, the gateway:

1. Downloads the file immediately from the protocol using the credential's auth.
2. Validates: rejects if file exceeds `max_file_size_mb` or MIME type is blocked/not allowed.
3. Stores in the local file cache with a unique ID and TTL.
4. Includes a gateway-hosted `download_url` in the normalized envelope.

The backend fetches the file via `GET /files/{file_id}` whenever it needs it. The file is always available within the TTL regardless of protocol-specific expiry or auth requirements.

Rejected files do not block the message. The attachment is replaced with a metadata stub:

```json
{
  "type": "rejected",
  "filename": "large_video.mp4",
  "mime_type": "video/mp4",
  "size_bytes": 157286400,
  "reason": "file_too_large"
}
```

Backend adapters that don't support files (e.g., OpenCode) simply drop the `attachments` field.

### 7.2 Outbound (Backend → Protocol)

The backend sends files as multipart form data in the `POST /api/v1/send` request. The gateway receives the file in memory and uploads it using the protocol's native method (multipart for Telegram, CDN upload for Discord, MIME attachment for email).

### 7.3 File Cache Configuration

| Field | Description |
|-------|-------------|
| `directory` | Local path for cached files |
| `ttl_hours` | Time before cached files are deleted |
| `max_cache_size_mb` | Total cache size limit — oldest files evicted first |
| `cleanup_interval_minutes` | How often the cleanup task runs |
| `max_file_size_mb` | Maximum size per individual file |
| `allowed_mime_types` | Whitelist of accepted MIME types, supports wildcards (e.g., `image/*`) |
| `blocked_mime_types` | Blacklist of rejected MIME types, evaluated after allowed list |

Validation order: file size check → blocked MIME type check → allowed MIME type check.

## 8. Emergency Mode

### 8.1 Health Monitoring

The gateway periodically checks configured health endpoints. If a check fails for `alert_after_failures` consecutive attempts:

1. Gateway enters **emergency mode** for that target.
2. All credentials marked `emergency: true` that route to the affected target receive a notification.
3. Incoming messages for the affected target are buffered in memory (configurable max buffer size).
4. When the target recovers, buffered messages are delivered in order and emergency contacts are notified.

### 8.2 Non-Emergency Credentials

During an outage, messages from non-emergency credentials are buffered silently or auto-replied with "System temporarily unavailable" (configurable).

## 9. Security

### 9.1 Secret Management

- **Inline** — value directly in config.json (acceptable for local/dev deployments).
- **Environment variable** — `${VAR_NAME}` syntax resolved at startup and reload.

The admin API never returns raw token values. GET endpoints redact tokens.

### 9.2 Transport Security

All REST communication between gateway and backends should use HTTPS in production. Webhook mode requires HTTPS with a valid TLS certificate.

### 9.3 Admin API Protection

The admin API is protected by a bearer token. In production, additionally restrict by IP allowlist or mTLS.

## 10. Retry and Buffering

### 10.1 Inbound Retry

If the POST to the backend fails (network error, 5xx), the gateway retries with exponential backoff: 1s, 2s, 4s, 8s, max 30s. After max retries, the message is buffered (if emergency mode) or dropped with a log entry.

### 10.2 Outbound Retry

If the protocol API rejects a send (rate limit, temporary error), the gateway retries with protocol-appropriate backoff. Permanent failures return an error response to the backend.

## 11. Observability

### 11.1 Structured Logging

All message events are logged with structured fields: credential_id, user protocol, backend protocol, direction, latency, status.

### 11.2 Metrics (Prometheus-compatible)

- `gateway_messages_inbound_total` (by user protocol, backend protocol, credential)
- `gateway_messages_outbound_total` (by user protocol, backend protocol, credential)
- `gateway_message_latency_seconds` (by direction, protocol)
- `gateway_health_check_status` (by target)
- `gateway_active_credentials` (by protocol)
- `gateway_buffer_size` (messages waiting during outage)
- `gateway_file_cache_size_bytes`
- `gateway_file_cache_count`

## 12. Python SDK

A thin Python client wrapping the gateway's REST API.

```python
from pipelit_gateway import GatewayClient

gw = GatewayClient(
    base_url="https://gateway.local:8080",
    send_token="...",
    admin_token="..."
)

# Outbound messaging
response = gw.send(
    credential_id="yao_telegram",
    chat_id="123456",
    text="PR merged.",
    reply_to_message_id="789"
)

# Outbound with file
response = gw.send(
    credential_id="yao_telegram",
    chat_id="123456",
    text="Here's the report",
    file_path="/tmp/report.pdf"
)

# Credential management
gw.credentials.list()
gw.credentials.create({...})
gw.credentials.activate("holly_telegram")
gw.credentials.deactivate("dev_discord")
gw.credentials.delete("old_credential")

# Health
gw.health()
```

## 13. CLI Tool

Built on the Python SDK for command-line management.

```bash
# Credential management
gw-cli credentials list
gw-cli credentials create --protocol telegram --mode poll \
    --token $TOKEN --workflow wf_abc --trigger trigger_tg
gw-cli credentials activate holly_telegram
gw-cli credentials deactivate dev_discord

# Send a test message
gw-cli send --credential yao_telegram --chat 123456 --text "test"

# Health check
gw-cli health
```

## 14. Open Questions

1. **Buffer persistence** — should buffered messages during outages survive a gateway restart? If yes, needs a write-ahead log or local SQLite.
2. **Multi-gateway** — can multiple gateway instances share the same config and coordinate? Probably not needed at family/small-team scale.
3. **Credential encryption at rest** — should tokens be encrypted in config.json beyond env var resolution?
4. **Rate limit coordination** — per-credential rate limiting is straightforward, but does the backend need to know about protocol rate limits to back-pressure output?
5. **OpenCode session lifecycle** — should the gateway create one session per credential, one per conversation, or reuse a single long-lived session?
