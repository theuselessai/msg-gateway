# msg-gateway Architecture

## Overview

msg-gateway is a message routing service that connects user-facing communication protocols to backend AI/LLM services.

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           msg-gateway                                    │
│                                                                          │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────────────────┐   │
│  │   Adapters   │    │    Core      │    │      Backends            │   │
│  │              │    │              │    │                          │   │
│  │  Telegram ───┼───▶│  Router      │───▶│  Pipelit (webhook)       │   │
│  │  Discord  ───┼───▶│  Health Mon  │───▶│  OpenCode (REST+SSE)     │   │
│  │  Slack    ───┼───▶│  File Cache  │    │                          │   │
│  │  Email    ───┼───▶│  Config      │    │                          │   │
│  │  Generic  ───┼───▶│              │    │                          │   │
│  └──────────────┘    └──────────────┘    └──────────────────────────┘   │
│         ▲                   │                        │                   │
│         │                   │                        │                   │
│         └───────────────────┴────────────────────────┘                   │
│                        /api/v1/send                                      │
└─────────────────────────────────────────────────────────────────────────┘
```

## Components

### Core Gateway (Rust)

The gateway core is written in Rust using the Axum web framework.

| Component | File | Purpose |
|-----------|------|---------|
| HTTP Server | `src/server.rs` | Routes, middleware, state management |
| Config | `src/config.rs` | Configuration loading, env var resolution |
| Router | `src/server.rs` | Message routing to backends |
| Health Monitor | `src/health.rs` | Backend health checks, message buffering |
| File Cache | `src/files.rs` | Attachment download/upload caching |
| Admin API | `src/admin.rs` | Credential CRUD operations |
| Adapter Manager | `src/adapter.rs` | External adapter process lifecycle |
| Credential Manager | `src/manager.rs` | Credential task registry |
| Config Watcher | `src/watcher.rs` | Hot reload on config changes |

### Adapters

Adapters translate between protocol-specific formats and the gateway's normalized message format.

#### Types

1. **Built-in Adapters**: Run in the gateway process
   - Generic (REST + WebSocket)

2. **External Adapters**: Run as separate processes
   - Telegram, Discord, Slack, Email
   - Managed by `AdapterInstanceManager`
   - Communicate via HTTP

#### External Adapter Lifecycle

```
1. Gateway starts
2. For each active credential:
   a. Find adapter definition in adapters_dir
   b. Allocate port from adapter_port_range
   c. Spawn subprocess with environment:
      - INSTANCE_ID
      - ADAPTER_PORT
      - GATEWAY_URL
      - CREDENTIAL_TOKEN
      - CREDENTIAL_CONFIG
   d. Wait for health check to pass
   e. Register in process map
3. On config change:
   a. Detect added/removed/changed credentials
   b. Stop removed adapters
   c. Start new adapters
   d. Restart changed adapters
```

### Backends

Backends receive normalized messages and send replies.

#### Pipelit Protocol
```
Inbound:  POST {inbound_url} with InboundMessage
Outbound: POST /api/v1/send from Pipelit to gateway
```

#### OpenCode Protocol
```
Inbound:  POST {base_url}/conversation with message
Outbound: SSE polling for responses
```

## Data Flow

### Inbound Message (User → Backend)

```
1. User sends message via protocol (e.g., Telegram)
2. External adapter receives message
3. Adapter POSTs to gateway /api/v1/adapter/inbound
4. Gateway normalizes message to InboundMessage format
5. Gateway checks health state:
   - If backend down: buffer message
   - If backend up: forward to backend
6. Gateway returns acknowledgment to adapter
```

### Outbound Message (Backend → User)

```
1. Backend POSTs to gateway /api/v1/send
2. Gateway validates auth token
3. Gateway looks up credential
4. For generic adapter:
   - Send via WebSocket to connected clients
5. For external adapter:
   - POST to adapter's /send endpoint
6. Adapter sends to user via protocol
7. Gateway returns protocol_message_id
```

## Message Format

### InboundMessage (Normalized)

```json
{
  "route": { ... },
  "credential_id": "my_telegram",
  "source": {
    "protocol": "telegram",
    "chat_id": "123456789",
    "message_id": "msg_001",
    "from": {
      "id": "user_123",
      "username": "johndoe",
      "display_name": "John Doe"
    }
  },
  "text": "Hello, assistant!",
  "attachments": [
    {
      "filename": "image.png",
      "mime_type": "image/png",
      "size_bytes": 12345,
      "download_url": "http://gateway/files/f_abc123"
    }
  ],
  "timestamp": "2026-03-08T12:00:00Z"
}
```

### Outbound Message (Send Request)

```json
{
  "credential_id": "my_telegram",
  "chat_id": "123456789",
  "text": "Hello, user!",
  "reply_to_message_id": "msg_001",
  "file": {
    "url": "http://backend/files/response.pdf",
    "filename": "response.pdf",
    "mime_type": "application/pdf"
  }
}
```

## Configuration

See [config.example.json](../config.example.json) for a full example.

### Key Sections

```json
{
  "gateway": {
    "listen": "0.0.0.0:8080",
    "admin_token": "${ADMIN_TOKEN}",
    "adapters_dir": "./adapters",
    "adapter_port_range": [9000, 9100],
    "default_target": { ... },
    "file_cache": { ... },
    "health_checks": { ... }
  },
  "auth": {
    "send_token": "${SEND_TOKEN}"
  },
  "credentials": {
    "credential_id": { ... }
  }
}
```

## Security

- Admin API requires `admin_token` in Authorization header
- Send API requires `send_token` in Authorization header
- Credential tokens are never exposed in API responses
- Environment variable references (`${VAR}`) resolved at load time
- File cache validates MIME types and size limits

## See Also

- [Adapter Protocol](adapters/protocol.md)
- [E2E Testing Guide](testing/e2e.md)
- [Roadmap](roadmap.md)
