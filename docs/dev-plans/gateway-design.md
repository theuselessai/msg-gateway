# Pipelit Gateway — Design Document

**Status:** Active
**Author:** Yao
**Date:** 2026-03-07
**Version:** 4

---

## 1. Overview

Pipelit Gateway is a standalone Rust binary that serves as a multi-protocol message bridge. It connects user-facing communication protocols (Telegram, Discord, Slack, Email, Generic HTTP/WS) to backend agent protocols (Pipelit, OpenCode).

The gateway is a protocol bridge — adapters on both sides, normalized messages in the middle. User-facing adapters are **external processes** (written in any language) that the gateway manages. Backend-facing adapters are built into the gateway. Adding a new messaging platform means writing an external adapter script; adding a new backend means implementing a Rust trait.

The gateway is framework-agnostic. Routing metadata (`route`) is an opaque key-value map passed through without interpretation. The gateway and backend servers are fully decoupled — either can be restarted, redeployed, or scaled independently.

## 2. Problem Statement

Pipelit's current architecture tightly couples protocol polling to individual workers, with each worker bound to a single credential. This creates three pain points:

1. **Multiple credential polling** — adding a new user (e.g., a family member) requires structural changes rather than configuration changes.
2. **Multiple protocol support** — each protocol (Telegram, Slack, Discord, Email) has different connection models (long-polling, WebSocket, IMAP) that don't fit cleanly into a single worker pattern.
3. **Outbound routing** — workflow responses need to reach the correct protocol and chat via the correct credential, requiring credential context to flow through the entire system.

## 3. Architecture

### 3.1 Protocol Bridge Model

The gateway bridges user-facing protocols to backend-facing protocols. Each credential spawns an external adapter process. The gateway manages adapter lifecycle and communicates via HTTP.

```
┌─────────────────────────────────────────────────────────────────┐
│                        Gateway (Rust)                            │
│                                                                  │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │
│  │ Config      │  │ Adapter     │  │ HTTP Server             │  │
│  │ Watcher     │  │ Manager     │  │                         │  │
│  │ (fsnotify)  │  │ (processes) │  │ /api/v1/send            │  │
│  │             │  │             │  │ /api/v1/adapter/inbound │  │
│  │             │  │             │  │ /files/{id}             │  │
│  │             │  │             │  │ /admin/...              │  │
│  │             │  │             │  │ /generic/...            │  │
│  └─────────────┘  └─────────────┘  └─────────────────────────┘  │
│                                                                  │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │
│  │ File Cache  │  │ Health      │  │ Backend Adapters        │  │
│  │ (temp dir)  │  │ Monitor     │  │ (Pipelit, OpenCode)     │  │
│  └─────────────┘  └─────────────┘  └─────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
         │                                      ▲
         │ spawn/manage                         │ POST /api/v1/adapter/inbound
         ▼                                      │
┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
│ Adapter Process │  │ Adapter Process │  │ Adapter Process │
│ (telegram)      │  │ (telegram)      │  │ (discord)       │
│ inst_001        │  │ inst_002        │  │ inst_003        │
│ port 9001       │  │ port 9002       │  │ port 9003       │
│                 │  │                 │  │                 │
│ credential:     │  │ credential:     │  │ credential:     │
│ yao_telegram    │  │ holly_telegram  │  │ dev_discord     │
└─────────────────┘  └─────────────────┘  └─────────────────┘
         │                   │                    │
         ▼                   ▼                    ▼
    Telegram API       Telegram API         Discord API
    (via MCP or        (via MCP or          (via MCP or
     direct)            direct)              direct)
```

### 3.2 Design Principles

- **Protocol bridge** — the gateway bridges user protocols to backend protocols. User-facing adapters are external processes; backend-facing adapters are built-in.
- **External adapters** — user-facing adapters are separate processes (any language). Gateway manages their lifecycle like Claude Desktop manages MCP servers.
- **Dumb routing** — one credential maps to one backend target. All routing intelligence lives in the backend's workflow/agent logic.
- **Config-driven** — credentials, targets, and routes are defined in a JSON config file. Adapters are discovered from a directory. Changes are detected via filesystem watching.
- **Decoupled** — gateway and backends communicate via their respective adapter protocols. No shared Redis, no shared filesystem.
- **One process per credential** — each credential spawns an independent adapter process. Isolation is the default; scaling is adding credentials.

## 4. Configuration

### 4.1 Directory Structure

```
msg-gateway/
├── config.json              # Gateway configuration
├── adapters/                # Adapter definitions
│   ├── telegram/
│   │   ├── adapter.json     # Adapter metadata
│   │   ├── main.py
│   │   └── requirements.txt
│   ├── discord/
│   │   ├── adapter.json
│   │   └── main.py
│   ├── slack/
│   │   ├── adapter.json
│   │   └── main.py
│   └── email/
│       ├── adapter.json
│       └── main.py
```

### 4.2 Adapter Definition (adapter.json)

Each adapter directory contains an `adapter.json` that tells the gateway how to run it:

```json
{
  "name": "telegram",
  "version": "0.1.0",
  "command": "python",
  "args": ["main.py"]
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Adapter identifier, matches directory name |
| `version` | string | yes | Adapter version |
| `command` | string | yes | Executable to run |
| `args` | array | no | Command line arguments |

### 4.3 Gateway Config (config.json)

```json
{
  "gateway": {
    "listen": "0.0.0.0:8080",
    "admin_token": "${GATEWAY_ADMIN_TOKEN}",
    "adapters_dir": "./adapters",
    "adapter_port_range": [9000, 9100],
    "default_target": {
      "protocol": "pipelit",
      "inbound_url": "https://pipelit.local/api/v1/inbound",
      "token": "${PIPELIT_API_TOKEN}"
    },
    "file_cache": {
      "directory": "/var/lib/gateway/files",
      "ttl_hours": 24,
      "max_cache_size_mb": 500,
      "cleanup_interval_minutes": 30,
      "max_file_size_mb": 50,
      "allowed_mime_types": ["text/*", "image/*", "application/pdf"],
      "blocked_mime_types": ["application/x-executable"]
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
      "adapter": "telegram",
      "token": "${YAO_TELEGRAM_TOKEN}",
      "active": true,
      "emergency": true,
      "config": {
        "poll_timeout": 30
      },
      "route": {
        "workflow_id": "wf_abc123",
        "trigger_id": "trigger_telegram_yao"
      }
    },
    "holly_telegram": {
      "adapter": "telegram",
      "token": "${HOLLY_TELEGRAM_TOKEN}",
      "active": true,
      "emergency": false,
      "route": {
        "workflow_id": "wf_def456",
        "trigger_id": "trigger_telegram_holly"
      }
    },
    "dev_discord": {
      "adapter": "discord",
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
    },
    "web_client": {
      "adapter": "generic",
      "token": "${WEB_CLIENT_TOKEN}",
      "active": true,
      "emergency": false,
      "route": {
        "workflow_id": "wf_web",
        "trigger_id": "trigger_web"
      }
    }
  }
}
```

### 4.4 Credential Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `adapter` | string | yes | Adapter name (must exist in adapters_dir), or `generic` for built-in |
| `token` | string | yes | Protocol auth token (passed to adapter via env var) |
| `active` | bool | yes | Whether this credential is currently enabled |
| `emergency` | bool | yes | Whether to notify via this credential when a health check fails |
| `config` | object | no | Adapter-specific configuration (passed to adapter) |
| `target` | object | no | Per-credential backend target override. If omitted, uses `gateway.default_target` |
| `route` | object | yes | Opaque key-value map passed through to the backend |

### 4.5 Backend Target Types

#### Pipelit (default)

Fire-and-forget POST. The backend receives the message and calls the gateway back when it has a response.

```json
{
  "protocol": "pipelit",
  "inbound_url": "https://pipelit.local/api/v1/inbound",
  "token": "${PIPELIT_API_TOKEN}"
}
```

#### OpenCode

Session-based interaction. The gateway manages sessions, sends messages via `prompt_async`, and polls for responses.

```json
{
  "protocol": "opencode",
  "base_url": "https://opencode.local:4096",
  "token": "${OPENCODE_TOKEN}",
  "poll_interval_ms": 500
}
```

OpenCode does not support file attachments. The OpenCode adapter drops the `attachments` field.

### 4.6 Auth Scopes

| Scope | Token Config Key | Protects |
|-------|-----------------|----------|
| Admin | `gateway.admin_token` | `/admin/*` endpoints |
| Send | `auth.send_token` | `/api/v1/send` and `/files/{file_id}` |
| Inbound | `target.token` | Outbound calls from gateway to backend |

## 5. External Adapter Protocol

### 5.1 Adapter Lifecycle

1. **Startup**: Gateway spawns adapter process with environment variables:
   ```bash
   INSTANCE_ID=inst_001
   ADAPTER_PORT=9001
   GATEWAY_URL=http://localhost:8080
   CREDENTIAL_ID=yao_telegram
   CREDENTIAL_TOKEN=xxx_telegram_bot_token
   CREDENTIAL_CONFIG='{"poll_timeout":30}'
   ```

2. **Ready**: Adapter starts HTTP server on assigned port. Gateway polls `/health` until ready.

3. **Running**: Adapter connects to external service (Telegram, Discord, etc.) and:
   - Receives messages → POSTs to Gateway
   - Receives send commands from Gateway → sends to external service

4. **Shutdown**: Gateway sends SIGTERM. Adapter gracefully disconnects and exits.

### 5.2 Gateway Maintains Instance Mapping

```
adapter_instances:
  inst_001:
    adapter: "telegram"
    credential_id: "yao_telegram"
    port: 9001
    pid: 12345
    status: "running"
  
  inst_002:
    adapter: "telegram"
    credential_id: "holly_telegram"
    port: 9002
    pid: 12346
    status: "running"
```

### 5.3 Adapter → Gateway (Inbound Message)

When the adapter receives a message from the external service:

```
POST http://{gateway_url}/api/v1/adapter/inbound
Content-Type: application/json

{
  "instance_id": "inst_001",
  "chat_id": "123456",
  "message_id": "789",
  "text": "Hello from user",
  "from": {
    "id": "user_001",
    "username": "yao",
    "display_name": "Yao"
  },
  "file": {
    "url": "https://api.telegram.org/file/...",
    "auth_header": "Bearer xxx",
    "filename": "photo.jpg",
    "mime_type": "image/jpeg"
  },
  "timestamp": "2026-03-07T10:30:00Z"
}
```

Gateway:
1. Looks up `instance_id` → `credential_id`
2. Downloads file if present, stores in cache
3. Builds normalized `InboundMessage` with `route` from credential config
4. Forwards to backend adapter (Pipelit or OpenCode)

**Response:** `202 Accepted`

### 5.4 Gateway → Adapter (Outbound Send)

When the backend sends a message via `/api/v1/send`:

```
POST http://localhost:{adapter_port}/send
Content-Type: application/json

{
  "chat_id": "123456",
  "text": "PR #102 merged. Coverage at 93%.",
  "reply_to_message_id": "789",
  "file_path": "/tmp/gateway/files/f_abc123.pdf"
}
```

Adapter sends to external service and responds:

```json
{
  "protocol_message_id": "telegram_msg_456"
}
```

### 5.5 Adapter Health Check

```
GET http://localhost:{adapter_port}/health

Response: 200 OK
{
  "status": "healthy"
}
```

## 6. Built-in Generic Adapter

The Generic protocol is built into the gateway (not an external process). It provides a REST + WebSocket interface for web/TUI clients.

### 6.1 Inbound (REST)

```
POST /generic/{credential_id}/chat
Authorization: Bearer {credential_token}
Content-Type: application/json

{
  "text": "Hello from web client",
  "chat_id": "session_123",
  "from": {
    "id": "user_001",
    "display_name": "Web User"
  }
}
```

### 6.2 Outbound (WebSocket)

```
WS /generic/{credential_id}/ws?chat_id={chat_id}
```

Messages are JSON:
```json
{
  "type": "message",
  "text": "Response from backend",
  "timestamp": "2026-03-07T10:30:05Z"
}
```

## 7. Backend-Facing Protocol Adapters

Backend adapters are built into the gateway and implement a common trait.

### 7.1 Backend Adapter Trait

```rust
#[async_trait]
pub trait BackendAdapter: Send + Sync {
    async fn send_message(&self, message: &InboundMessage) -> Result<(), BackendError>;
    fn supports_files(&self) -> bool;
}
```

### 7.2 Pipelit Adapter

- POSTs normalized message to `inbound_url`
- Returns immediately after 202 response
- Response comes back via `/api/v1/send` callback
- Supports file attachments (via `download_url`)

### 7.3 OpenCode Adapter

- Manages session per credential
- Sends via `POST /session/:id/prompt_async`
- Polls for response via messages API or SSE
- Does NOT support file attachments
- Delivers response directly to user-facing adapter

## 8. API Specification

### 8.1 Inbound from Adapter

```
POST /api/v1/adapter/inbound
Content-Type: application/json
```

See §5.3 for request format.

### 8.2 Outbound from Backend

```
POST /api/v1/send
Authorization: Bearer {auth.send_token}
Content-Type: application/json (or multipart/form-data with file)

{
  "credential_id": "yao_telegram",
  "chat_id": "123456",
  "reply_to_message_id": "789",
  "text": "PR #102 merged."
}
```

Gateway looks up `credential_id` → `instance_id` → `port`, then POSTs to adapter's `/send`.

### 8.3 File Download

```
GET /files/{file_id}
Authorization: Bearer {auth.send_token}
```

Returns binary file with appropriate headers. **404** if unknown, **410 Gone** if expired.

### 8.4 Admin Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /admin/credentials` | List all credentials with status |
| `GET /admin/credentials/:id` | Get single credential |
| `POST /admin/credentials` | Create credential, spawn adapter |
| `PUT /admin/credentials/:id` | Update credential, restart adapter if needed |
| `PATCH /admin/credentials/:id/activate` | Activate and spawn adapter |
| `PATCH /admin/credentials/:id/deactivate` | Deactivate and stop adapter |
| `DELETE /admin/credentials/:id` | Delete credential, stop adapter |
| `GET /admin/health` | Gateway status, adapter states |

## 9. File Handling

### 9.1 Inbound (Adapter → Gateway → Backend)

1. Adapter receives message with file from external service
2. Adapter extracts file URL and auth info, includes in POST to gateway:
   ```json
   {
     "file": {
       "url": "https://api.telegram.org/file/...",
       "auth_header": "Bearer xxx",
       "filename": "photo.jpg",
       "mime_type": "image/jpeg"
     }
   }
   ```
3. Gateway downloads file, validates size/MIME, stores in cache
4. Gateway includes `download_url` in normalized message to backend
5. Backend fetches via `GET /files/{file_id}` when needed

### 9.2 Outbound (Backend → Gateway → Adapter)

1. Backend POSTs to `/api/v1/send` with multipart file
2. Gateway stores file temporarily
3. Gateway POSTs to adapter with `file_path`
4. Adapter uploads to external service

## 10. Emergency Mode

### 10.1 Health Monitoring

The gateway periodically checks configured health endpoints. If a check fails for `alert_after_failures` consecutive attempts:

1. Gateway enters **emergency mode** for that target
2. All credentials marked `emergency: true` receive a notification
3. Incoming messages are buffered in memory
4. When target recovers, buffered messages are delivered

### 10.2 Non-Emergency Credentials

During an outage, messages from non-emergency credentials are buffered silently.

## 11. Security

### 11.1 Secret Management

- **Environment variable** — `${VAR_NAME}` syntax resolved at startup
- Credential tokens passed to adapters via env vars (never in command line)
- Admin API never returns raw token values

### 11.2 Transport Security

All REST communication should use HTTPS in production.

## 12. Observability

### 12.1 Structured Logging

All message events logged with: credential_id, adapter, backend protocol, direction, latency, status.

### 12.2 Metrics (Prometheus-compatible)

- `gateway_messages_inbound_total`
- `gateway_messages_outbound_total`
- `gateway_message_latency_seconds`
- `gateway_adapter_status` (by adapter, credential)
- `gateway_health_check_status`
- `gateway_buffer_size`
- `gateway_file_cache_size_bytes`

## 13. Python SDK

```python
from pipelit_gateway import GatewayClient

gw = GatewayClient(
    base_url="https://gateway.local:8080",
    send_token="...",
    admin_token="..."
)

# Send message
gw.send(credential_id="yao_telegram", chat_id="123456", text="Hello")

# With file
gw.send(credential_id="yao_telegram", chat_id="123456", text="Report", file_path="/tmp/report.pdf")

# Credential management
gw.credentials.list()
gw.credentials.create({...})
gw.credentials.activate("holly_telegram")
```

## 14. CLI Tool

```bash
plit credentials list
plit credentials create --adapter telegram --token $TOKEN --route '{"workflow_id":"wf_abc"}'
plit send --credential yao_telegram --chat 123456 --text "test"
plit health
```

## 15. Open Questions

1. **Buffer persistence** — should buffered messages survive gateway restart?
2. **Adapter crash recovery** — auto-restart adapters? How many retries?
3. **Credential encryption at rest** — encrypt tokens in config.json?
4. **Rate limit coordination** — how to handle protocol rate limits?
5. **OpenCode session lifecycle** — one session per credential or per conversation?
