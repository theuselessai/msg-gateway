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
  "file_paths": ["/tmp/cache/f_abc123.pdf"],
  "extra_data": {}
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `chat_id` | string | Yes | Platform-specific chat/channel ID |
| `text` | string | Yes | Message text (may be empty if file only) |
| `reply_to_message_id` | string | No | Platform message ID to reply to |
| `file_paths` | string[] | No | Local paths to file attachments (downloaded by gateway from file cache) |
| `extra_data` | object | No | Protocol-specific data (see [Extra Data by Protocol](#extra-data-by-protocol)) |

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
  "reply_to_message_id": "tg_msg_67889",
  "text": "Hello, assistant!",
  "from": {
    "id": "user_456",
    "username": "johndoe",
    "display_name": "John Doe"
  },
  "timestamp": "2026-03-08T12:00:00Z",
  "files": [
    {
      "url": "https://api.telegram.org/file/...",
      "filename": "photo.jpg",
      "mime_type": "image/jpeg",
      "auth_header": "Bot 123:ABC..."
    }
  ],
  "extra_data": {}
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `instance_id` | string | Yes | Value from INSTANCE_ID env var |
| `chat_id` | string | Yes | Platform-specific chat/channel ID |
| `message_id` | string | Yes | Platform-specific message ID |
| `reply_to_message_id` | string | No | Platform message ID that the user replied to |
| `text` | string | Yes | Message text (may be empty) |
| `from.id` | string | Yes | Platform-specific user ID |
| `from.username` | string | No | Username if available |
| `from.display_name` | string | No | Display name if available |
| `timestamp` | string | No | ISO 8601 timestamp (defaults to now) |
| `files` | object[] | No | File attachments (multiple supported) |
| `files[].url` | string | Yes* | URL to download the file |
| `files[].filename` | string | Yes* | Original filename |
| `files[].mime_type` | string | Yes* | MIME type of the file |
| `files[].auth_header` | string | No | Authorization header for download |
| `extra_data` | object | No | Protocol-specific data (see [Extra Data by Protocol](#extra-data-by-protocol)) |

\* Required when `files` array is present.

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
  const { chat_id, text, reply_to_message_id, file_paths, extra_data } = request.body as any;
  
  // TODO: Send via platform API (handle file_paths and extra_data as needed)
  const messageId = await sendToPlatform(chat_id, text, {
    replyTo: reply_to_message_id,
    files: file_paths,
    extra: extra_data,
  });
  
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
      reply_to_message_id: message.replyToId || undefined,
      text: message.text,
      from: {
        id: message.from.id,
        username: message.from.username,
        display_name: message.from.displayName,
      },
      files: message.attachments?.map((a: any) => ({
        url: a.url,
        filename: a.filename,
        mime_type: a.mimeType,
        auth_header: a.authHeader,
      })),
      extra_data: message.extraData || undefined,
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

## Gateway Send API (Backend → Gateway → Adapter)

When a backend sends a message through the gateway, it uses `POST /api/v1/send`:

```json
{
  "credential_id": "my_telegram",
  "chat_id": "123456789",
  "text": "Here is the report you requested.",
  "reply_to_message_id": "tg_msg_67890",
  "file_ids": ["f_abc123", "f_def456"],
  "extra_data": {}
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `credential_id` | string | Yes | Which credential/adapter to send through |
| `chat_id` | string | Yes | Platform-specific chat/channel ID |
| `text` | string | Yes | Message text (may be empty if file only) |
| `reply_to_message_id` | string | No | Platform message ID to reply to |
| `file_ids` | string[] | No | IDs of files uploaded via [File Upload API](../api/files.md) |
| `extra_data` | object | No | Protocol-specific data (see below) |

The gateway resolves `file_ids` to local file paths (downloading from cache) and forwards the message to the adapter's `POST /send` endpoint with `file_paths` instead of `file_ids`.

## Extra Data by Protocol

The `extra_data` field carries protocol-specific metadata. The gateway passes it through transparently — adapters populate it on inbound, and backends can include it on outbound. The backend/LLM interprets the contents.

### Telegram

No `extra_data` needed. All relevant fields are covered by the core message format (`chat_id`, `message_id`, `reply_to_message_id`, `files`).

```json
{
  "extra_data": {}
}
```

### Discord

```json
{
  "extra_data": {
    "thread_id": "1234567890",
    "guild_id": "9876543210",
    "channel_name": "general"
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `thread_id` | string | Discord thread ID (if message is in a thread) |
| `guild_id` | string | Discord server (guild) ID |
| `channel_name` | string | Human-readable channel name |

### Slack

```json
{
  "extra_data": {
    "thread_ts": "1234567890.123456",
    "channel_name": "general",
    "team_id": "T01234567"
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `thread_ts` | string | Slack thread timestamp (if message is in a thread) |
| `channel_name` | string | Human-readable channel name |
| `team_id` | string | Slack workspace ID |

### Email

```json
{
  "extra_data": {
    "subject": "Re: Project Update",
    "to": ["alice@example.com"],
    "cc": ["bob@example.com"],
    "in_reply_to": "<msg-id@example.com>",
    "references": ["<prev-msg@example.com>"],
    "html_body": "<p>Hello</p>",
    "is_cc": false,
    "is_bcc": false
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `subject` | string | Email subject line |
| `to` | string[] | Recipient email addresses |
| `cc` | string[] | CC recipients |
| `in_reply_to` | string | Message-ID of the email being replied to |
| `references` | string[] | Message-ID chain for threading |
| `html_body` | string | HTML body (inbound only; `text` has plain text) |
| `is_cc` | boolean | Whether this credential was CC'd (inbound only) |
| `is_bcc` | boolean | Whether this credential was BCC'd (inbound only) |

### Generic

No `extra_data` needed. The generic adapter uses the core message format directly via REST and WebSocket.

```json
{
  "extra_data": {}
}
```

## See Also

- [Architecture](../architecture.md)
- [E2E Testing](../testing/e2e.md)
- [File Upload API](../api/files.md)
