# msg-gateway Architecture

## Overview

msg-gateway is a protocol bridge that connects user-facing communication platforms to backend AI/LLM services. User-facing adapters run as external subprocesses (except the built-in Generic adapter). Backend adapters can be either built-in (Rust implementations) or external (subprocess-managed, any language). The gateway normalizes messages between both sides.

## System Architecture

```
                          ┌─────────────────────────────────────────────────────────────┐
                          │                      msg-gateway (Rust / Axum)              │
                          │                                                             │
  External Adapters       │  ┌───────────────────────────────────────────────────────┐  │     Backend Services
  (subprocesses)          │  │                    HTTP Server                        │  │
                          │  │                                                       │  │
┌──────────────────┐      │  │  POST /api/v1/adapter/inbound   (adapter → gateway)  │  │
│ Telegram Adapter │─────▶│  │  POST /api/v1/send              (backend → gateway)  │  │
│ (Node.js)        │◀─────│  │  POST /api/v1/files             (file upload)        │  │
│ port 9001        │      │  │  GET  /files/{id}               (file download)      │  │
└──────────────────┘      │  │  POST /api/v1/chat/{cred}       (generic inbound)    │  │
                          │  │  WS   /ws/chat/{cred}/{chat}    (generic outbound)   │  │   ┌──────────────────┐
┌──────────────────┐      │  │  GET  /health                   (health check)       │  │   │    Pipelit       │
│ Discord Adapter  │─────▶│  │  CRUD /admin/credentials/*      (admin API)          │  ├──▶│    (webhook)     │
│ (Node.js)        │◀─────│  │                                                       │  │   │                  │
│ port 9002        │      │  └───────────────────────────────────────────────────────┘  │   │ POST inbound_url │
└──────────────────┘      │                                                             │   └──────────────────┘
                          │  ┌─────────────┐ ┌─────────────┐ ┌───────────────────────┐  │
┌──────────────────┐      │  │ Adapter     │ │ Config      │ │ Health Monitor        │  │   ┌──────────────────┐
│ Slack Adapter    │─────▶│  │ Manager     │ │ Watcher     │ │                       │  ├──▶│    OpenCode      │
│ (Node.js)        │◀─────│  │             │ │ (fsnotify)  │ │ state: HealthState    │  │   │    (REST+SSE)    │
│ port 9003        │      │  │ spawn/stop/ │ │             │ │ buffer: VecDeque      │  │   │                  │
└──────────────────┘      │  │ health check│ │ hot reload  │ │ emergency alerts      │  │   │ POST /conversation│
                          │  └─────────────┘ └─────────────┘ └───────────────────────┘  │   └──────────────────┘
┌──────────────────┐      │                                                             │
│ Email Adapter    │─────▶│  ┌─────────────┐ ┌─────────────┐ ┌───────────────────────┐  │
│ (Node.js)        │◀─────│  │ Credential  │ │ File Cache  │ │ Generic Adapter       │  │
│ port 9004        │      │  │ Manager     │ │             │ │ (built-in)            │  │
└──────────────────┘      │  │             │ │ download/   │ │                       │  │
                          │  │ registry of │ │ store/serve │ │ REST inbound          │  │
                          │  │ instances   │ │ cleanup/TTL │ │ WebSocket outbound    │  │
                          │  └─────────────┘ └─────────────┘ └───────────────────────┘  │
                          │                                                             │
                          └─────────────────────────────────────────────────────────────┘
```

## Domain Model

### Core Entities

```
┌─────────────────────────────────────────────────────────────────────────────────────┐
│                                    AppState                                         │
│                              (shared across all routes)                              │
├─────────────────────────────────────────────────────────────────────────────────────┤
│                                                                                     │
│  ┌──────────────────────────┐    ┌──────────────────────────────────────────────┐   │
│  │         Config           │    │          AdapterInstanceManager              │   │
│  ├──────────────────────────┤    ├──────────────────────────────────────────────┤   │
│  │ gateway: GatewayConfig   │    │ adapters: Map<name, AdapterDef>             │   │
│  │   listen: String         │    │ processes: Map<cred_id, AdapterProcess>     │   │
│  │   admin_token: String    │    │ port_allocator: PortAllocator              │   │
│  │   adapters_dir: String   │    │ gateway_url: String                        │   │
│  │   adapter_port_range     │    └──────────────────────────────────────────────┘   │
│  │   default_target ────────┼──┐                                                   │
│  │   file_cache: Option ────┼──┼──┐  ┌──────────────────────────────────────────┐  │
│  │ auth: AuthConfig         │  │  │  │        CredentialManager                 │  │
│  │   send_token: String     │  │  │  ├──────────────────────────────────────────┤  │
│  │ health_checks: Map       │  │  │  │ registry: TaskRegistry                  │  │
│  │ credentials: Map ────────┼┐ │  │  │   instances: Map<cred_id, InstanceInfo> │  │
│  └──────────────────────────┘│ │  │  └──────────────────────────────────────────┘  │
│                              │ │  │                                                 │
│  ┌───────────────────────┐   │ │  │  ┌───────────────────────────────────────────┐  │
│  │  CredentialConfig     │◀──┘ │  │  │         HealthMonitor                    │  │
│  ├───────────────────────┤     │  │  ├───────────────────────────────────────────┤  │
│  │ adapter: String       │     │  │  │ state: HealthState                       │  │
│  │ token: String         │     │  │  │   Healthy | Degraded | Down | Recovering │  │
│  │ active: bool          │     │  │  │ failure_count: u32                       │  │
│  │ emergency: bool       │     │  │  │ buffer: VecDeque<InboundMessage>         │  │
│  │ config: Option<JSON>  │     │  │  │ max_buffer_size: usize                  │  │
│  │ target: Option ───────┼──┐  │  │  └───────────────────────────────────────────┘  │
│  │ route: JSON           │  │  │  │                                                 │
│  └───────────────────────┘  │  │  │  ┌───────────────────────────────────────────┐  │
│                             │  │  └─▶│           FileCache                      │  │
│  ┌───────────────────────┐  │  │     ├───────────────────────────────────────────┤  │
│  │   TargetConfig        │◀─┴──┘     │ config: FileCacheConfig                  │  │
│  ├───────────────────────┤           │ files: Map<file_id, CachedFile>          │  │
│  │ protocol:             │           │ base_url: String                         │  │
│  │   Pipelit | Opencode  │           └───────────────────────────────────────────┘  │
│  │ inbound_url: Option   │                                                         │
│  │ base_url: Option      │     ┌─────────────────────────────────────────────────┐  │
│  │ token: String         │     │           WsRegistry                            │  │
│  │ poll_interval_ms: Opt │     │  Map<(cred_id, chat_id), broadcast::Sender>     │  │
│  └───────────────────────┘     └─────────────────────────────────────────────────┘  │
│                                                                                     │
└─────────────────────────────────────────────────────────────────────────────────────┘
```

### Message Types

```
  Inbound (User → Gateway → Backend)          Outbound (Backend → Gateway → User)
  ────────────────────────────────────         ────────────────────────────────────

  ┌───────────────────────────┐               ┌───────────────────────────┐
  │     InboundMessage        │               │     OutboundMessage       │
  ├───────────────────────────┤               ├───────────────────────────┤
  │ route: JSON               │               │ credential_id: String     │
  │ credential_id: String     │               │ chat_id: String           │
  │ source ───────────────────┼──┐            │ reply_to_message_id: Opt  │
  │ text: String              │  │            │ text: String              │
  │ attachments: Vec ─────────┼──┼──┐         │ file_ids: Vec<String>  *  │
  │ timestamp: DateTime       │  │  │         │ extra_data: JSON       *  │
  └───────────────────────────┘  │  │         └───────────────────────────┘
                                 │  │                     │
  ┌───────────────────────────┐  │  │                     │ resolved by gateway
  │     MessageSource         │◀─┘  │                     ▼
  ├───────────────────────────┤     │         ┌───────────────────────────┐
  │ protocol: String          │     │         │   AdapterSendRequest      │
  │ chat_id: String           │     │         ├───────────────────────────┤
  │ message_id: String        │     │         │ chat_id: String           │
  │ from ─────────────────────┼──┐  │         │ text: String              │
  └───────────────────────────┘  │  │         │ reply_to_message_id: Opt  │
                                 │  │         │ file_paths: Vec<String> * │
  ┌───────────────────────────┐  │  │         │ extra_data: JSON        * │
  │       UserInfo            │◀─┘  │         └───────────────────────────┘
  ├───────────────────────────┤     │
  │ id: String                │     │         ┌───────────────────────────┐
  │ username: Option          │     │         │   WsOutboundMessage       │
  │ display_name: Option      │     │         ├───────────────────────────┤
  └───────────────────────────┘     │         │ text: String              │
                                    │         │ timestamp: DateTime       │
  ┌───────────────────────────┐     │         │ message_id: String        │
  │      Attachment           │◀────┘         └───────────────────────────┘
  ├───────────────────────────┤
  │ filename: String          │                * = v0.2.0 additions
  │ mime_type: String         │
  │ size_bytes: u64           │
  │ download_url: String      │
  └───────────────────────────┘
```

### Adapter Process Model

```
  ┌───────────────────────────┐         ┌───────────────────────────┐
  │      AdapterDef           │         │     AdapterProcess        │
  │    (from adapter.json)    │         │   (running instance)      │
  ├───────────────────────────┤         ├───────────────────────────┤
  │ name: String              │    ┌───▶│ instance_id: String       │
  │ version: String           │    │    │ credential_id: String     │
  │ command: String           │    │    │ adapter_name: String      │
  │ args: Vec<String>         │    │    │ port: u16                 │
  └───────────────────────────┘    │    │ process: Child            │
           │                       │    │ health: AdapterHealth     │
           │ spawns                │    │   Starting | Healthy      │
           ▼                       │    │   Unhealthy | Dead        │
  ┌───────────────────────────┐    │    │ consecutive_failures: u32 │
  │ AdapterInstanceManager    │    │    │ restart_count: u32        │
  ├───────────────────────────┤    │    │ token: String             │
  │ adapters: Map<name, Def>  │    │    │ config: Option<JSON>      │
  │ processes: Map<cred, Proc>│────┘    └───────────────────────────┘
  │ port_allocator ───────────┼──┐
  │ gateway_url: String       │  │      ┌───────────────────────────┐
  └───────────────────────────┘  │      │     PortAllocator         │
                                 └─────▶├───────────────────────────┤
                                        │ range_start: u16          │
                                        │ range_end: u16            │
                                        │ allocated: Vec<u16>       │
                                        └───────────────────────────┘
```

### Backend Adapter Trait

```
                          ┌──────────────────────────────────┐
                          │     BackendAdapter (trait)        │
                          ├──────────────────────────────────┤
                          │ send_message(&InboundMessage)    │
                          │ supports_files() -> bool         │
                          └──────────┬───────────────────────┘
                                     │
                       ┌─────────────┴─────────────┐
                       │                           │
          ┌────────────────────────┐   ┌────────────────────────┐
          │   PipelitAdapter      │   │   OpencodeAdapter      │
          ├────────────────────────┤   ├────────────────────────┤
          │ client: reqwest        │   │ client: reqwest        │
          │ inbound_url: String    │   │ base_url: String       │
          │ token: String          │   │ token: String          │
          │                        │   │ poll_interval_ms: u64  │
          │ supports_files = true  │   │ supports_files = false │
          └────────────────────────┘   └────────────────────────┘
```

### Error Hierarchy

```
  HTTP Layer                              Backend Layer

  ┌──────────────────────────┐            ┌──────────────────────────┐
  │       AppError           │            │      BackendError        │
  ├──────────────────────────┤            ├──────────────────────────┤
  │ Config(String)      500  │            │ Network(reqwest)         │
  │ Unauthorized        401  │            │ BackendResponse{status}  │
  │ CredentialNotFound  404  │            │ InvalidConfig(String)    │
  │ CredentialInactive  400  │            │ Timeout                  │
  │ NotFound(String)    404  │            └──────────────────────────┘
  │ Gone(String)        410  │
  │ Internal(String)    500  │
  └──────────────────────────┘
```

## Message Flow

### Inbound: User → Backend

```
  User                 Adapter              Gateway                    Backend
   │                    │                     │                          │
   │  send message      │                     │                          │
   ├───────────────────▶│                     │                          │
   │                    │  POST /api/v1/      │                          │
   │                    │  adapter/inbound    │                          │
   │                    ├────────────────────▶│                          │
   │                    │                     │  validate instance_id    │
   │                    │                     │  lookup credential       │
   │                    │                     │  resolve target          │
   │                    │                     │                          │
   │                    │                     │  [if files present]      │
   │                    │                     │  download & cache files  │
   │                    │                     │                          │
   │                    │                     │  normalize to            │
   │                    │                     │  InboundMessage          │
   │                    │                     │                          │
   │                    │                     │  [if backend healthy]    │
   │                    │                     │  POST inbound_url ──────▶│
   │                    │                     │                          │
   │                    │                     │  [if backend down]       │
   │                    │                     │  buffer message          │
   │                    │                     │  [if emergency cred]     │
   │                    │                     │  send alert to user      │
   │                    │                     │                          │
   │                    │    202 Accepted     │                          │
   │                    │◀───────────────────┤                          │
   │                    │                     │                          │
```

### Outbound: Backend → User

```
  Backend              Gateway                    Adapter              User
   │                    │                          │                    │
   │  POST /api/v1/send │                          │                    │
   ├───────────────────▶│                          │                    │
   │                    │  validate send_token     │                    │
   │                    │  lookup credential       │                    │
   │                    │                          │                    │
   │                    │  [if file_ids present]   │                    │
   │                    │  resolve to file_paths   │                    │
   │                    │                          │                    │
   │                    │  [if generic adapter]    │                    │
   │                    │  send via WebSocket ─────┼───────────────────▶│
   │                    │                          │                    │
   │                    │  [if external adapter]   │                    │
   │                    │  POST /send ────────────▶│                    │
   │                    │                          │  send via platform │
   │                    │                          ├───────────────────▶│
   │                    │                          │                    │
   │                    │  protocol_message_id     │                    │
   │                    │◀─────────────────────────┤                    │
   │  SendResponse      │                          │                    │
   │◀───────────────────┤                          │                    │
   │                    │                          │                    │
```

### File Upload Flow (Backend → User with files)

```
  Backend              Gateway                    Adapter              User
   │                    │                          │                    │
   │  POST /api/v1/files│                          │                    │
   │  (multipart)       │                          │                    │
   ├───────────────────▶│                          │                    │
   │                    │  validate token          │                    │
   │                    │  validate mime/size      │                    │
   │                    │  store in FileCache      │                    │
   │  {file_id: "f_.."}│                          │                    │
   │◀───────────────────┤                          │                    │
   │                    │                          │                    │
   │  POST /api/v1/send │                          │                    │
   │  {file_ids:[..]}   │                          │                    │
   ├───────────────────▶│                          │                    │
   │                    │  resolve file_ids →      │                    │
   │                    │  file_paths              │                    │
   │                    │  POST /send              │                    │
   │                    │  {file_paths:[..]} ─────▶│                    │
   │                    │                          │  upload to platform│
   │                    │                          ├───────────────────▶│
   │                    │                          │                    │
```

### Config Hot Reload

```
  Filesystem            Config Watcher           Gateway
   │                       │                      │
   │  config.json modified │                      │
   ├──────────────────────▶│                      │
   │                       │  debounce (1s)       │
   │                       │  parse new config    │
   │                       │  diff credentials    │
   │                       │                      │
   │                       │  [added credentials] │
   │                       │  spawn adapters ─────▶│
   │                       │                      │
   │                       │  [removed creds]     │
   │                       │  stop adapters ──────▶│
   │                       │                      │
   │                       │  [changed creds]     │
   │                       │  restart adapters ───▶│
   │                       │                      │
   │                       │  update config ──────▶│
   │                       │                      │
```

## Components

### Core Gateway (Rust)

| Component | File | Purpose |
|-----------|------|---------|
| HTTP Server | `src/server.rs` | Routes, middleware, state management |
| Config | `src/config.rs` | Configuration loading, env var resolution |
| Messages | `src/message.rs` | Normalized message types |
| Health Monitor | `src/health.rs` | Backend health checks, message buffering |
| File Cache | `src/files.rs` | Attachment download/upload caching |
| Admin API | `src/admin.rs` | Credential CRUD operations |
| Adapter Manager | `src/adapter.rs` | External adapter process lifecycle |
| Credential Manager | `src/manager.rs` | Credential task registry |
| Config Watcher | `src/watcher.rs` | Hot reload on config changes |
| Backend Adapters | `src/backend.rs` | Pipelit and OpenCode protocol adapters |
| Generic Adapter | `src/generic.rs` | built-in REST + WebSocket adapter |
| Errors | `src/error.rs` | Error types and HTTP status mapping |

### Adapters

1. **built-in**: Generic adapter (REST inbound + WebSocket outbound), runs in-process
2. **External**: Telegram, Discord, Slack, Email — separate Node.js processes managed by gateway

External adapters communicate with the gateway via:
- `POST /send` — gateway tells adapter to send a message
- `GET /health` — gateway checks adapter health
- `POST /api/v1/adapter/inbound` — adapter forwards inbound messages to gateway

### Backends

| Backend | Protocol | Inbound | Outbound |
|---------|----------|---------|----------|
| Pipelit | Webhook + callback | `POST {inbound_url}` with `InboundMessage` | `POST /api/v1/send` from backend |
| OpenCode | REST + SSE | `POST {base_url}/conversation` | SSE polling for responses |

### Backend Adapter Models

The gateway supports two backend adapter models:

#### 1. Built-in Backends (Rust)

Built-in backends are Rust struct implementations of the `BackendAdapter` trait, compiled directly into the gateway binary. They run in-process as part of the gateway.

**Available built-in backends:**
- `BackendProtocol::Pipelit` → `PipelitAdapter` (HTTP client for Pipelit webhooks)
- `BackendProtocol::Opencode` → `OpencodeAdapter` (HTTP client + SSE for OpenCode)

**Singleton-per-name model:** One adapter instance per named backend entry in `config.backends`, shared across all credentials referencing that backend.

```
// Example: All credentials using "opencode" backend share one OpencodeAdapter instance
config.backends:
  opencode  →  ONE OpencodeAdapter instance (in-process Rust)

Credentials:
  telegram  (backend: opencode)  ──┐
  slack     (backend: opencode)  ──┼──▶  single OpencodeAdapter handles all
  discord   (backend: opencode)  ──┘     via internal session map (credential:chat → sessionId)
```

#### 2. External Backends (Subprocess)

External backends (`BackendProtocol::External`) run as separate subprocesses, managed by `ExternalBackendManager`. They can be written in any language and communicate via HTTP.

**Singleton-per-name model:** One subprocess per named backend entry in `config.backends`, shared across all credentials referencing that backend.

```
config.backends:
  opencode  →  ONE Node.js process (port 9200, backends/opencode/)
  pipelit   →  ONE Node.js process (port 9201, backends/pipelit/)

Credentials:
  telegram  (backend: opencode)  ──┐
  slack     (backend: opencode)  ──┼──▶  opencode subprocess handles all
  discord   (backend: opencode)  ──┘     via internal routing/session management
```

**Protocol:** External backends receive env vars (`BACKEND_PORT`, `GATEWAY_URL`, `BACKEND_TOKEN`, `BACKEND_CONFIG`) and expose:
- `POST /send` — Receive messages from gateway
- `GET /health` — Health check endpoint

#### Current Limitation

The singleton-per-name model (both built-in and external) only works when all credentials sharing a backend name talk to the **same upstream instance** (same base URL, same auth token). If credentials need to reach different upstream endpoints or isolated accounts, a single shared adapter/process cannot serve them all correctly.

**When this breaks down:**
- Multi-tenant deployments where each user has their own OpenCode server
- Per-credential Pipelit workspace tokens pointing to different endpoints
- Any scenario where backend config differs per credential, not per backend name

**Future direction:** Per-credential backend isolation — one adapter instance (or subprocess) per credential, with `CREDENTIAL_CONFIG` carrying per-credential endpoint/token (same pattern as external adapters). See roadmap.

## Security

- Admin API requires `admin_token` in Authorization header
- Send API requires `send_token` in Authorization header
- Credential tokens are never exposed in API responses
- Environment variable references (`${VAR}`) resolved at config load time
- File cache validates MIME types and enforces size limits
- File IDs are unguessable UUIDs

## See Also

- [Adapter Protocol](adapters/protocol.md) — External adapter HTTP protocol spec
- [File Upload API](api/files.md) — File upload/download endpoints
- [E2E Testing Guide](testing/e2e.md) — End-to-end test framework
- [Roadmap](roadmap.md) — Release plans
