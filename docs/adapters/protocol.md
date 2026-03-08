# Adapter Protocol Specification

This document defines the protocol for external adapters communicating with msg-gateway.

## Overview

External adapters are standalone processes that:
1. Connect to a messaging platform (Telegram, Discord, etc.)
2. Receive messages from users
3. Forward messages to the gateway
4. Receive replies from the gateway
5. Send replies back to users

## Environment Variables

The gateway provides these environment variables when starting an adapter:

| Variable | Description | Example |
|----------|-------------|---------|
| `INSTANCE_ID` | Unique identifier for this adapter instance | `telegram_abc123` |
| `ADAPTER_PORT` | Port the adapter must listen on | `9001` |
| `GATEWAY_URL` | Base URL of the gateway | `http://127.0.0.1:8080` |
| `CREDENTIAL_ID` | ID of the credential this adapter serves | `my_telegram` |
| `CREDENTIAL_TOKEN` | Authentication token for the messaging platform | `bot123:ABC...` |
| `CREDENTIAL_CONFIG` | JSON string of adapter-specific config | `{"chat_ids": [...]}` |

## Required Endpoints

Adapters must implement these HTTP endpoints:

### GET /health

Health check endpoint called by the gateway.

**Response:**
```json
{
  "status": "ok"
}
```

**Status Codes:**
- `200`: Adapter is healthy
- `503`: Adapter is unhealthy

### POST /send

Send a message to a user.

**Request:**
```json
{
  "chat_id": "123456789",
  "text": "Hello, user!",
  "reply_to_message_id": "msg_001",
  "file_path": "/tmp/cache/f_abc123.pdf"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `chat_id` | string | Yes | Platform-specific chat/channel ID |
| `text` | string | Yes | Message text (may be empty if file only) |
| `reply_to_message_id` | string | No | ID of message to reply to |
| `file_path` | string | No | Local path to file attachment |

**Response:**
```json
{
  "protocol_message_id": "tg_msg_12345"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `protocol_message_id` | string | Platform-specific message ID |

**Status Codes:**
- `200`: Message sent successfully
- `400`: Invalid request
- `500`: Failed to send

## Gateway Callback

Adapters must POST inbound messages to the gateway.

### POST {GATEWAY_URL}/api/v1/adapter/inbound

**Request:**
```json
{
  "instance_id": "telegram_abc123",
  "chat_id": "123456789",
  "message_id": "tg_msg_67890",
  "text": "Hello, assistant!",
  "from": {
    "id": "user_456",
    "username": "johndoe",
    "display_name": "John Doe"
  },
  "timestamp": "2026-03-08T12:00:00Z",
  "file": {
    "url": "https://api.telegram.org/file/...",
    "filename": "photo.jpg",
    "mime_type": "image/jpeg",
    "auth_header": "Bot 123:ABC..."
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `instance_id` | string | Yes | Value from INSTANCE_ID env var |
| `chat_id` | string | Yes | Platform-specific chat/channel ID |
| `message_id` | string | Yes | Platform-specific message ID |
| `text` | string | Yes | Message text (may be empty) |
| `from.id` | string | Yes | Platform-specific user ID |
| `from.username` | string | No | Username if available |
| `from.display_name` | string | No | Display name if available |
| `timestamp` | string | No | ISO 8601 timestamp (defaults to now) |
| `file` | object | No | File attachment info |
| `file.url` | string | Yes* | URL to download the file |
| `file.filename` | string | Yes* | Original filename |
| `file.mime_type` | string | Yes* | MIME type of the file |
| `file.auth_header` | string | No | Authorization header for download |

**Response:**
```json
{
  "status": "accepted"
}
```

**Status Codes:**
- `202`: Message accepted
- `400`: Invalid request
- `401`: Unknown instance_id
- `500`: Internal error

## Adapter Definition File

Each adapter directory must contain an `adapter.json` file:

```json
{
  "name": "telegram",
  "command": "node",
  "args": ["dist/main.js"],
  "health_check": {
    "path": "/health",
    "interval_ms": 5000,
    "timeout_ms": 3000,
    "retries": 3
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Adapter name (matches directory name) |
| `command` | string | Yes | Executable to run |
| `args` | string[] | No | Command line arguments |
| `health_check.path` | string | No | Health endpoint path (default: `/health`) |
| `health_check.interval_ms` | number | No | Check interval (default: 5000) |
| `health_check.timeout_ms` | number | No | Request timeout (default: 3000) |
| `health_check.retries` | number | No | Retries before unhealthy (default: 3) |

## Directory Structure

```
adapters/
├── telegram/
│   ├── adapter.json
│   ├── package.json
│   ├── tsconfig.json
│   └── src/
│       └── main.ts
├── discord/
│   ├── adapter.json
│   └── ...
└── slack/
    ├── adapter.json
    └── ...
```

## TypeScript Template

```typescript
import Fastify from 'fastify';

const app = Fastify();
const PORT = parseInt(process.env.ADAPTER_PORT!);
const GATEWAY_URL = process.env.GATEWAY_URL!;
const INSTANCE_ID = process.env.INSTANCE_ID!;
const TOKEN = process.env.CREDENTIAL_TOKEN!;

// Health check
app.get('/health', async () => ({ status: 'ok' }));

// Send message to user
app.post('/send', async (request, reply) => {
  const { chat_id, text, reply_to_message_id, file_path } = request.body as any;
  
  // TODO: Send via platform API
  const messageId = await sendToplatform(chat_id, text);
  
  return { protocol_message_id: messageId };
});

// Forward inbound message to gateway
async function forwardToGateway(message: any) {
  await fetch(`${GATEWAY_URL}/api/v1/adapter/inbound`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      instance_id: INSTANCE_ID,
      chat_id: message.chatId,
      message_id: message.id,
      text: message.text,
      from: {
        id: message.from.id,
        username: message.from.username,
        display_name: message.from.displayName,
      },
    }),
  });
}

app.listen({ port: PORT, host: '0.0.0.0' });
```

## Error Handling

### Startup Failures

If an adapter fails to start or health check fails after retries:
1. Gateway logs error
2. Credential marked as unhealthy
3. Gateway continues with other credentials
4. Periodic restart attempts based on config

### Message Delivery Failures

If `/send` returns error:
1. Gateway logs error
2. Error returned to backend
3. Backend can retry if needed

### Gateway Unavailable

If gateway is unreachable when forwarding inbound:
1. Adapter should retry with exponential backoff
2. After max retries, drop message and log error
3. Platform-specific: may acknowledge to prevent redelivery

## See Also

- [Architecture](../architecture.md)
- [E2E Testing](../testing/e2e.md)
