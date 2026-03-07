# Pipelit Gateway — Design Document

**Status:** Draft
**Author:** Yao
**Date:** 2026-03-07

---

## 1. Overview

Pipelit Gateway is a standalone Rust binary that serves as a multi-protocol message gateway. While designed for the Pipelit agent orchestration platform, it is framework-agnostic — any backend that accepts and sends HTTP requests can use it. The gateway handles inbound message ingestion from multiple communication protocols, outbound message delivery, credential lifecycle management, and health monitoring with emergency alerting.

The gateway is intentionally stateless and agnostic to the downstream server's architecture. Routing metadata (`route`) is an opaque key-value map defined in configuration and passed through to the target server without interpretation. It communicates with the target server exclusively via REST APIs. The two services are fully decoupled — either can be restarted, redeployed, or scaled independently.

## 2. Problem Statement

Pipelit's current architecture tightly couples protocol polling to individual workers, with each worker bound to a single credential. This creates three pain points:

1. **Multiple credential polling** — adding a new user (e.g., a family member) requires structural changes rather than configuration changes.
2. **Multiple protocol support** — each protocol (Telegram, Slack, Discord, Email) has different connection models (long-polling, WebSocket, IMAP, webhooks) that don't fit cleanly into a single worker pattern.
3. **Outbound routing** — workflow responses need to reach the correct protocol and chat via the correct credential, requiring credential context to flow through the entire system.

## 3. Architecture

### 3.1 System Context

```
┌─────────────────────────────────────────────────────────────┐
│                       Gateway (Rust)                         │
│                                                              │
│  ┌──────────────┐  ┌──────────────┐  ┌───────────────────┐  │
│  │ Config       │  │ Protocol     │  │ HTTP Server        │  │
│  │ Watcher      │  │ Adapters     │  │                    │  │
│  │ (fsnotify)   │  │              │  │ POST /webhook/...  │  │
│  │              │  │ ┌──────────┐ │  │ POST /api/v1/send  │  │
│  │ config.json ─┤  │ │telegram  │ │  │ GET  /files/{id}   │  │
│  │              │  │ │slack     │ │  │ /admin/credentials │  │
│  │              │  │ │slack     │ │  │ /admin/health      │  │
│  └──────────────┘  │ │discord   │ │  └────────┬──────────┘  │
│                    │ │email     │ │           │             │
│                    │ │generic   │ │           │             │
│                    │ └──────────┘ │           │             │
│                    └──────┬───────┘           │             │
│                           │                   │             │
└───────────────────────────┼───────────────────┼─────────────┘
                            │                   │
                   POST /api/v1/inbound         │
                            │                   │
                            ▼                   │
                ┌───────────────────────┐       │
                │   Pipelit Server      │       │
                │   (Python/FastAPI)    │───────┘
                │                       │  POST /api/v1/send
                │   Workflows           │
                │   Sandboxes           │
                │   Redis + RQ          │
                └───────────────────────┘
```

### 3.2 Design Principles

- **Stateless** — the gateway holds no session state. All conversation context lives in Pipelit's workflow engine.
- **Dumb routing** — one credential maps to one workflow + trigger. All routing intelligence lives in Pipelit's workflow nodes (categorizer, router, switch).
- **Config-driven** — credentials and routes are defined in a JSON config file. Changes are detected via filesystem watching and applied without restart.
- **Decoupled** — gateway and Pipelit communicate exclusively via REST. No shared Redis, no shared filesystem.
- **One task per credential** — each credential spawns an independent async task (tokio). Isolation is the default; scaling is adding credentials.

## 4. Configuration

### 4.1 Config File Format

The gateway reads from a single JSON config file, watched via fsnotify for live changes. Secret values can reference environment variables using `${ENV_VAR}` syntax.

```json
{
  "gateway": {
    "listen": "0.0.0.0:8080",
    "admin_token": "${GATEWAY_ADMIN_TOKEN}",
    "pipelit": {
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
    "family_discord": {
      "protocol": "discord",
      "mode": "poll",
      "token": "${FAMILY_DISCORD_TOKEN}",
      "active": false,
      "emergency": false,
      "route": {
        "workflow_id": "wf_ghi789",
        "trigger_id": "trigger_discord_family"
      }
    },
    "generic_chat": {
      "protocol": "generic",
      "token": "${GENERIC_CHAT_TOKEN}",
      "active": true,
      "emergency": false,
      "route": {
        "workflow_id": "wf_local",
        "trigger_id": "trigger_generic"
      }
    }
  }
}
```

The `route` object is opaque to the gateway. For Pipelit, it contains `workflow_id` and `trigger_id`. Another framework might use:

```json
{
  "route": {
    "agent_id": "assistant_main"
  }
}
```

### 4.2 Environment Variable Resolution

If a config value starts with `${` and ends with `}`, the gateway resolves it from the process environment at startup and on config reload. Missing env vars cause a startup error for required fields.

### 4.3 Credential Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `protocol` | string | yes | Protocol type: `telegram`, `slack`, `discord`, `email`, `generic` |
| `mode` | string | yes | Connection mode: `poll` or `webhook` |
| `token` | string | yes | Protocol auth token (or env var reference) |
| `active` | bool | yes | Whether this credential is currently enabled |
| `emergency` | bool | yes | Whether to notify via this credential when Pipelit is down |
| `webhook_path` | string | webhook mode | URL path the gateway registers for inbound webhooks |
| `route` | object | yes | Opaque key-value map passed through to the target server. The gateway does not interpret route contents — the receiving server defines the schema. For Pipelit: `{"workflow_id": "...", "trigger_id": "..."}`. For other frameworks: `{"agent_id": "..."}` or any custom schema. |

### 4.4 Auth Scopes

Three authentication scopes protect the gateway's endpoints:

| Scope | Token Config Key | Protects |
|-------|-----------------|----------|
| Admin | `gateway.admin_token` | `GET/POST/PUT/DELETE /admin/*` |
| Send | `auth.send_token` | `POST /api/v1/send` (Pipelit → Gateway) |
| Inbound | `gateway.pipelit.token` | `POST /api/v1/inbound` (Gateway → Pipelit) |

All tokens are passed as `Authorization: Bearer <token>` headers.

## 5. API Specification

### 5.1 Inbound Message (Gateway → Pipelit)

The gateway POSTs normalized messages to Pipelit when a message arrives from any protocol.

```
POST {gateway.pipelit.inbound_url}
Authorization: Bearer {gateway.pipelit.token}
Content-Type: application/json
```

**Request Body:**

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

**Response:** `202 Accepted` — the target server acknowledges receipt. The `route` field is passed through as-is from the credential config — the gateway does not interpret its contents.

### 5.2 Outbound Message (Pipelit → Gateway)

Pipelit POSTs outbound messages to the gateway when a workflow produces a response. Messages without attachments use JSON. Messages with attachments use multipart form data.

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

The gateway resolves `credential_id` to the correct protocol adapter and auth token, then delivers the message. Pipelit receives confirmation that the message was sent, including the protocol's native message ID for threading.

### 5.3 File Download (Target Server → Gateway)

The target server fetches inbound attachments from the gateway's file endpoint. Files are downloaded and cached by the gateway at inbound receive time, so they are always available regardless of protocol-specific expiry or auth requirements.

```
GET /files/{file_id}
Authorization: Bearer {auth.send_token}
```

**Response:** Binary file stream with appropriate `Content-Type` and `Content-Disposition` headers.

**404** if the file ID is unknown. **410 Gone** if the file has expired (TTL exceeded).

#### File Cache Configuration

```json
{
  "gateway": {
    "file_cache": {
      "directory": "/var/lib/gateway/files",
      "ttl_hours": 24,
      "max_cache_size_mb": 500,
      "cleanup_interval_minutes": 30,
      "max_file_size_mb": 50,
      "allowed_mime_types": ["text/plain", "text/markdown", "text/csv", "application/pdf", "application/json"],
      "blocked_mime_types": ["application/x-executable", "application/x-msdownload", "..."]
    }
  }
}
```

| Field | Description |
|-------|-------------|
| `directory` | Local path for cached files |
| `ttl_hours` | Time before cached files are deleted |
| `max_cache_size_mb` | Total cache size limit — oldest files evicted first |
| `cleanup_interval_minutes` | How often the cleanup task runs |
| `max_file_size_mb` | Maximum size per individual file. Files exceeding this are rejected at download time — the inbound message is still delivered but with the attachment replaced by a metadata stub indicating the file was too large. |
| `allowed_mime_types` | Whitelist of accepted MIME types. Supports wildcards (e.g., `image/*`). If set, only matching types are cached. |
| `blocked_mime_types` | Blacklist of rejected MIME types. Evaluated after allowed list. Blocks executables, scripts, and other dangerous types by default. |

**Validation order:** file size check → blocked MIME type check → allowed MIME type check. Rejected files produce a log entry and an attachment stub in the inbound message:

```json
{
  "type": "rejected",
  "filename": "malware.exe",
  "mime_type": "application/x-msdownload",
  "size_bytes": 15728640,
  "reason": "blocked_mime_type"
}
```

### 5.4 Generic Chat API

The `generic` protocol provides a REST + WebSocket interface for clients that don't use third-party protocols (e.g., web apps, TUI clients, local scripts). Unlike other protocols where the gateway connects to external services, `generic` exposes endpoints that clients connect to directly.

#### Inbound (Client → Gateway → Pipelit)

```
POST /api/v1/chat/{credential_id}
Authorization: Bearer {credential.token}
Content-Type: application/json
```

**Request Body:**

```json
{
  "chat_id": "session_abc123",
  "text": "Hello from my TUI app",
  "from": {
    "id": "local_user",
    "display_name": "Yao"
  }
}
```

**Response:** `202 Accepted` — fire and forget. The gateway forwards the message to Pipelit and returns immediately.

```json
{
  "message_id": "msg_xyz789",
  "timestamp": "2026-03-07T10:30:00Z"
}
```

The `chat_id` is client-managed (e.g., UUID generated by the client). It identifies a conversation session for routing outbound responses.

#### Outbound (Pipelit → Gateway → Client via WebSocket)

Clients establish a WebSocket connection to receive responses:

```
GET /ws/chat/{credential_id}/{chat_id}
Authorization: Bearer {credential.token}
Upgrade: websocket
```

The gateway maintains a registry of active WebSocket connections keyed by `(credential_id, chat_id)`. When Pipelit sends an outbound message via `POST /api/v1/send`, the gateway looks up the matching WebSocket and pushes the message:

```json
{
  "text": "Response from Pipelit",
  "timestamp": "2026-03-07T10:30:05Z",
  "message_id": "msg_abc123"
}
```

If no WebSocket is connected for the target `chat_id`, the message is dropped with a log entry (fire and forget — no buffering for generic protocol).

#### Flow Diagram

```
┌─────────┐         ┌─────────────┐         ┌─────────────┐
│ Client  │         │   Gateway   │         │   Pipelit   │
│(TUI/Web)│         │             │         │             │
└────┬────┘         └──────┬──────┘         └──────┬──────┘
     │                     │                       │
     │ ① WS Connect        │                       │
     │ /ws/chat/generic_chat/session_123          │
     │────────────────────>│                       │
     │     101 Switching   │                       │
     │<────────────────────│                       │
     │                     │                       │
     │ ② POST /api/v1/chat/generic_chat           │
     │   {chat_id, text}   │                       │
     │────────────────────>│                       │
     │     202 Accepted    │                       │
     │<────────────────────│                       │
     │                     │                       │
     │                     │ ③ POST /api/v1/inbound
     │                     │──────────────────────>│
     │                     │     202 Accepted      │
     │                     │<──────────────────────│
     │                     │                       │
     │                     │ ④ POST /api/v1/send   │
     │                     │   {credential_id,     │
     │                     │    chat_id, text}     │
     │                     │<──────────────────────│
     │                     │                       │
     │ ⑤ WS Push           │                       │
     │ {text: "response"}  │                       │
     │<────────────────────│                       │
```

#### Configuration

```json
{
  "generic_chat": {
    "protocol": "generic",
    "token": "${GENERIC_CHAT_TOKEN}",
    "active": true,
    "emergency": false,
    "route": {
      "workflow_id": "wf_local",
      "trigger_id": "trigger_generic"
    }
  }
}
```

The `token` field authenticates both the REST inbound endpoint and the WebSocket connection. Unlike other protocols, `generic` does not require `mode` — it always uses REST for inbound and WebSocket for outbound.

### 5.5 Admin Endpoints

#### List Credentials

```
GET /admin/credentials
Authorization: Bearer {gateway.admin_token}
```

Returns all credentials with their status (active/inactive, connection state).

#### Get Credential

```
GET /admin/credentials/:id
Authorization: Bearer {gateway.admin_token}
```

Returns a single credential's config and runtime status. Token values are redacted in responses.

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

Updates a credential's config. Restarts the adapter task if protocol/mode/token changed.

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

Returns gateway status: uptime, active credentials, connection states, Pipelit reachability.

## 6. Protocol Adapters

Each protocol adapter is an independent async task (tokio) that handles the protocol-specific connection and message normalization.

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
| `poll` | Adapter spawns an async loop that polls the protocol API at a configured interval. Used when webhooks aren't available or practical (e.g., no public URL). |
| `webhook` | Adapter registers an HTTP route on the gateway's server at `webhook_path`. No polling — the protocol pushes messages to the gateway. |

### 6.3 Supported Protocols

| Protocol | Poll | Webhook | Rust Crate | Notes |
|----------|------|---------|------------|-------|
| Telegram | Long polling | Bot API webhooks | `teloxide` | One connection per bot token |
| Slack | — | Events API | `slack-morphism` | Webhook-only; requires URL verification |
| Discord | WebSocket gateway | Interactions endpoint | `serenity` | WebSocket is the standard model |
| Email | IMAP IDLE | — | `async-imap` | Poll via IMAP IDLE (push-like) |
| Generic | — | REST + WebSocket | `axum`, `tokio-tungstenite` | HTTP inbound, WebSocket outbound; for web/TUI clients |

### 6.4 Message Normalization

All adapters produce the same normalized message envelope regardless of protocol. Protocol-specific fields (e.g., Telegram's `chat_type`, Discord's `guild_id`) are discarded at the gateway — Pipelit only sees the normalized format.

Attachments are normalized by downloading at receive time. Each protocol handles files differently (Telegram uses file_id, Slack requires auth, Discord CDN URLs expire, email embeds files as MIME parts), so the gateway resolves these differences at inbound time by downloading the file immediately and caching it locally.

**Inbound:** The gateway downloads the file from the protocol at receive time and stores it in a local temp cache with a configurable TTL. The normalized message envelope includes a gateway-hosted download URL. The target server fetches files from this URL whenever it needs them — the file is always available within the TTL regardless of protocol-specific expiry or auth requirements.

**Outbound:** The target server sends files directly to the gateway as multipart form data in the send request. The gateway receives the file in memory and uploads it using the protocol's native method (multipart for Telegram, CDN upload for Discord, MIME attachment for email). No storage needed for outbound.

## 7. Emergency Mode

### 7.1 Health Monitoring

The gateway periodically checks Pipelit's health endpoint. If the check fails for `alert_after_failures` consecutive attempts:

1. Gateway enters **emergency mode**.
2. All credentials marked `emergency: true` receive a notification: "Pipelit is unreachable. Messages are being buffered. Last healthy: {timestamp}."
3. Incoming messages from all credentials are buffered in memory (with a configurable max buffer size).
4. When Pipelit recovers, buffered messages are delivered in order and emergency contacts are notified: "Pipelit recovered. {N} buffered messages delivered."

### 7.2 Non-Emergency Credentials

During an outage, messages from non-emergency credentials can be configured to either buffer silently or return an auto-reply ("System temporarily unavailable").

## 8. Security

### 8.1 Secret Management

Protocol tokens and API keys are sensitive. The gateway supports two patterns:

- **Inline** — value directly in config.json (acceptable for local/dev deployments).
- **Environment variable** — `${VAR_NAME}` syntax resolved at startup and reload.

The admin API never returns raw token values. GET endpoints redact tokens to `"***...***"`.

### 8.2 Transport Security

All REST communication between gateway and Pipelit should use HTTPS in production. Webhook mode requires HTTPS with a valid TLS certificate (most protocols mandate this).

### 8.3 Admin API Protection

The admin API is protected by a bearer token. In production, it should additionally be restricted by IP allowlist or mTLS.

## 9. Retry and Buffering

### 9.1 Inbound Retry

If the POST to Pipelit's inbound endpoint fails (network error, 5xx), the gateway retries with exponential backoff: 1s, 2s, 4s, 8s, max 30s. After max retries, the message is buffered (if emergency mode) or dropped with a log entry.

### 9.2 Outbound Retry

If the protocol API rejects a send (rate limit, temporary error), the gateway retries with protocol-appropriate backoff. Permanent failures (invalid chat_id, credential revoked) return an error response to Pipelit.

## 10. Observability

### 10.1 Structured Logging

All message events are logged with structured fields: credential_id, protocol, direction (inbound/outbound), workflow_id, latency, status.

### 10.2 Metrics

Key metrics to expose (Prometheus-compatible):

- `gateway_messages_inbound_total` (by protocol, credential)
- `gateway_messages_outbound_total` (by protocol, credential)
- `gateway_message_latency_seconds` (by direction, protocol)
- `gateway_pipelit_health_status` (0/1)
- `gateway_active_credentials` (by protocol)
- `gateway_buffer_size` (messages waiting during outage)

## 11. Python SDK

A thin Python client wrapping the gateway's REST API. Used by Pipelit for outbound messages and by operators for credential management.

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

# Credential management
gw.credentials.list()
gw.credentials.create({...})
gw.credentials.activate("holly_telegram")
gw.credentials.deactivate("family_discord")
gw.credentials.delete("old_credential")

# Health
gw.health()
```

## 12. CLI Tool

Built on the Python SDK for command-line management.

```bash
# Credential management
gw-cli credentials list
gw-cli credentials create --protocol telegram --mode poll \
    --token $TOKEN --workflow wf_abc --trigger trigger_tg
gw-cli credentials activate holly_telegram
gw-cli credentials deactivate family_discord
gw-cli credentials delete old_credential

# Send a test message
gw-cli send --credential yao_telegram --chat 123456 --text "test"

# Health check
gw-cli health
```

## 13. Design Decisions

1. **Buffer persistence** — No. Buffered messages during Pipelit outages are in-memory only and lost on gateway restart. The gateway is a stateless message forwarder, not a message store. If persistence is needed, it belongs in Pipelit or an external queue.

2. **Multi-gateway** — No. Single instance is sufficient for family/small-team scale. No coordination protocol needed.

3. **Credential encryption at rest** — No. Use `${ENV_VAR}` references for sensitive values. Tokens should not be stored directly in config.json in production.

4. **Rate limit coordination** — No. Gateway handles per-credential rate limiting internally. On rate limit errors, it returns an error response to Pipelit. Pipelit can implement its own back-pressure logic if needed.

5. **Generic protocol message buffering** — No. If WebSocket is disconnected when an outbound message arrives, the message is dropped. The gateway does not buffer messages for disconnected clients. Clients requiring reliable delivery should use protocols with built-in persistence (Telegram, Discord, etc.).
