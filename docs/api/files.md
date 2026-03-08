# File Upload & Download API

This document describes the file upload and download endpoints used by backends to attach files to outbound messages.

## Overview

The file flow for outbound messages (backend → user) works as follows:

1. Backend uploads file(s) via `POST /api/v1/files`
2. Backend receives `file_id` for each uploaded file
3. Backend includes `file_ids` array in `POST /api/v1/send` request
4. Gateway resolves `file_ids` to local file paths
5. Gateway passes `file_paths` to the adapter's `POST /send` endpoint
6. Adapter sends file(s) to the user via the platform API

For inbound messages (user → backend), adapters include `files[]` with download URLs. The gateway downloads and caches these files, then forwards `file_ids` (or download URLs) to the backend.

## Endpoints

### POST /api/v1/files

Upload a file to the gateway's file cache. Returns a `file_id` that can be referenced in send requests.

**Authentication:** `Authorization: Bearer <send_token>`

**Content-Type:** `multipart/form-data`

**Request:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `file` | binary | Yes | The file to upload |
| `filename` | string | Yes | Original filename (e.g. `report.pdf`) |
| `mime_type` | string | No | MIME type (auto-detected if omitted) |

**Example:**

```bash
curl -X POST http://localhost:8080/api/v1/files \
  -H "Authorization: Bearer $SEND_TOKEN" \
  -F "file=@report.pdf" \
  -F "filename=report.pdf" \
  -F "mime_type=application/pdf"
```

**Response (200):**

```json
{
  "file_id": "f_abc123def456",
  "filename": "report.pdf",
  "mime_type": "application/pdf",
  "size_bytes": 102400,
  "download_url": "http://localhost:8080/files/f_abc123def456"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `file_id` | string | Unique ID to reference this file in send requests |
| `filename` | string | Original filename |
| `mime_type` | string | MIME type of the file |
| `size_bytes` | number | File size in bytes |
| `download_url` | string | URL to download the file |

**Error Responses:**

| Status | Description |
|--------|-------------|
| `400` | Missing required fields or invalid MIME type |
| `401` | Invalid or missing auth token |
| `413` | File exceeds `max_file_size_mb` limit |
| `415` | MIME type is blocked or not in allowed list |
| `500` | Internal error (disk write failure, etc.) |

### GET /files/{file_id}

Download a cached file by ID.

**Authentication:** None required (file IDs are unguessable).

**Response (200):**
- Body: raw file content
- `Content-Type`: the file's MIME type
- `Content-Disposition`: `attachment; filename="<original_filename>"`

**Error Responses:**

| Status | Description |
|--------|-------------|
| `404` | File not found |
| `410` | File expired (TTL exceeded) |

**Example:**

```bash
curl -O http://localhost:8080/files/f_abc123def456
```

## Configuration

File cache behavior is controlled by the `gateway.file_cache` section in `config.json`:

```json
{
  "gateway": {
    "file_cache": {
      "directory": "/var/lib/gateway/files",
      "ttl_hours": 24,
      "max_cache_size_mb": 500,
      "max_file_size_mb": 50,
      "cleanup_interval_minutes": 30,
      "allowed_mime_types": ["text/*", "image/*", "application/pdf"],
      "blocked_mime_types": ["application/x-executable"]
    }
  }
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `directory` | string | required | Directory path for cached files |
| `ttl_hours` | number | `24` | How long files are kept before cleanup |
| `max_cache_size_mb` | number | `500` | Maximum total cache size |
| `max_file_size_mb` | number | `50` | Maximum size per file |
| `cleanup_interval_minutes` | number | `30` | How often to run cleanup |
| `allowed_mime_types` | string[] | `["*/*"]` | Allowed MIME type patterns (supports wildcards like `image/*`) |
| `blocked_mime_types` | string[] | `[]` | Blocked MIME types (checked before allowed list) |

## Usage in Send Requests

After uploading files, reference them by `file_id` in the send request:

```json
{
  "credential_id": "my_telegram",
  "chat_id": "123456789",
  "text": "Here are the reports you requested.",
  "file_ids": ["f_abc123def456", "f_ghi789jkl012"]
}
```

The gateway resolves each `file_id` to its cached file path and passes `file_paths` to the adapter:

```json
{
  "chat_id": "123456789",
  "text": "Here are the reports you requested.",
  "file_paths": ["/var/lib/gateway/files/f_abc123def456.pdf", "/var/lib/gateway/files/f_ghi789jkl012.xlsx"]
}
```

## See Also

- [Adapter Protocol](../adapters/protocol.md) - How adapters handle files
- [Architecture](../architecture.md) - System overview
