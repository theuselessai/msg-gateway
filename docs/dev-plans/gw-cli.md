# `gw-cli` CLI Tool — Development Plan

**Status:** Ready
**Blocked by:** None (gateway API endpoints all exist)
**Priority:** P0 — needed for Pipelit integration testing and agent tooling

---

## Summary

A standalone Rust CLI binary (`gw-cli`) for interacting with the msg-gateway. The gateway server binary is `gw-server`. Supports interactive chat, one-shot message sending, WebSocket listening, credential management, and health checks. Designed as a unix-philosophy tool: pipe-friendly, JSON-native, and usable by both humans and AI agents.

## Requirements

1. **Unix-philosophy compatible** — stdin/stdout/stderr, proper exit codes, pipe-friendly
2. **Single tool** — standalone binary, no runtime dependencies
3. **TUI embeddable** — callable as subprocess from TUI applications
4. **Agent-usable** — structured JSON output, non-interactive modes for programmatic access

## Command Structure

### Chat Commands (Generic Adapter)

These commands work regardless of what backend the credential is routed to — they talk to the gateway's generic adapter interface.

```bash
# Interactive REPL — connect WS + send loop
gw-cli chat <credential_id> --chat-id <id>

# One-shot send (pipe-friendly)
gw-cli send <credential_id> --chat-id <id> --text "hello"
echo "hello" | gw-cli send <credential_id> --chat-id <id>
gw-cli send <credential_id> --chat-id <id> < message.txt

# Listen only — stream JSONL to stdout
gw-cli listen <credential_id> --chat-id <id>
```

### Admin Commands

```bash
# Credential management
gw-cli credentials list
gw-cli credentials create <id> --adapter <type> --token <tok> \
  --backend <name> --route '{"workflow_slug":"...","trigger_node_id":"..."}'
gw-cli credentials activate <id>
gw-cli credentials deactivate <id>

# Health check
gw-cli health
```

### I/O Behavior

| Mode | stdin | stdout | stderr | Exit code |
|------|-------|--------|--------|-----------|
| `gw-cli chat` | interactive input | formatted responses | connection status | 0 |
| `gw-cli send` | reads text if no `--text` | JSON result | errors | 0/1 |
| `gw-cli listen` | — | JSONL stream (one msg per line) | connection status | 0/1 |
| `gw-cli credentials list` | — | JSON array | errors | 0/1 |
| `gw-cli health` | — | JSON status | errors | 0/1 |

### Global Flags

```
--gateway-url <url>      (env: GATEWAY_URL, default: http://localhost:8080)
--token <token>          (env: GATEWAY_TOKEN — credential token for chat/send/listen)
--admin-token <token>    (env: GATEWAY_ADMIN_TOKEN — for credentials/health commands)
--send-token <token>     (env: GATEWAY_SEND_TOKEN — not typically needed by CLI users)
--json                   (force JSON output; auto-enabled when stdout is not a TTY)
--no-color               (disable colored output)
```

## Architecture

### How It Works

The CLI is purely a gateway client. It doesn't know about backends (Pipelit, OpenCode, etc.).

```
gw-cli send → POST /api/v1/chat/{credential_id}       → Gateway routes to backend
gw-cli listen → WS /ws/chat/{credential_id}/{chat_id}  → Gateway pushes responses
gw-cli chat  → send + listen combined in interactive REPL
gw-cli credentials → /admin/credentials/*              → Gateway admin API
gw-cli health → GET /health                            → Gateway health endpoint
```

### Auth Flow

- **chat/send/listen**: Uses the credential's token (Bearer auth against the generic adapter endpoint)
- **credentials/health**: Uses the admin token (Bearer auth against the admin API)

### Auto-detection

- When stdout is a TTY → human-readable output with colors
- When stdout is piped → JSON output (JSONL for streams)
- `--json` flag overrides to always JSON
- `--no-color` flag disables colors even on TTY

## Project Structure

```
msg-gateway/
├── Cargo.toml              # workspace root (add gw-cli member)
├── crates/
│   └── gw-cli/
│       ├── Cargo.toml      # binary crate
│       └── src/
│           ├── main.rs     # clap entry point
│           ├── commands/
│           │   ├── mod.rs
│           │   ├── chat.rs         # interactive REPL
│           │   ├── send.rs         # one-shot send
│           │   ├── listen.rs       # WS stream listener
│           │   ├── credentials.rs  # admin CRUD
│           │   └── health.rs       # health check
│           ├── client.rs           # HTTP + WS gateway client
│           └── output.rs           # JSON / human-readable formatter
└── src/                    # existing gateway code (unchanged)
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| `clap` (derive) | CLI argument parsing |
| `reqwest` + `rustls-tls` | HTTP client (matches gateway, no OpenSSL) |
| `tokio-tungstenite` + `rustls` | WebSocket client |
| `tokio` (full) | Async runtime |
| `serde` / `serde_json` | JSON serialization |
| `chrono` | Timestamp formatting |
| `colored` or `owo-colors` | Terminal colors (optional) |
| `atty` or `std::io::IsTerminal` | TTY detection |

## Implementation Phases

### Phase 1 — Project Setup & Client Foundation

- [ ] Create workspace layout (`Cargo.toml` workspace, `crates/gw-cli/`)
- [ ] Set up `clap` with subcommands skeleton
- [ ] Implement `client.rs` — HTTP client for gateway API
  - `send_chat_message(credential_id, chat_id, text, token)` → POST /api/v1/chat/{cred}
  - `list_credentials(admin_token)` → GET /admin/credentials
  - `create_credential(...)` → POST /admin/credentials
  - `activate_credential(id)` → PATCH /admin/credentials/{id}/activate
  - `deactivate_credential(id)` → PATCH /admin/credentials/{id}/deactivate
  - `health_check()` → GET /health
- [ ] Implement `output.rs` — TTY-aware JSON/human formatter
- [ ] Global flag handling (env vars, defaults)

### Phase 2 — Core Commands

- [ ] `gw-cli send` — one-shot message send
  - Read from `--text` flag or stdin
  - POST to generic chat endpoint
  - Print JSON result to stdout
  - Proper exit codes
- [ ] `gw-cli listen` — WebSocket stream
  - Connect to WS endpoint with auth
  - Stream JSONL to stdout (one `WsOutboundMessage` per line)
  - Auto-reconnect on disconnect (with backoff)
  - Clean shutdown on SIGINT/SIGTERM
- [ ] `gw-cli health` — health check
  - GET /health
  - Print JSON status
  - Exit 0 if healthy, 1 if unhealthy

### Phase 3 — Interactive Chat

- [ ] `gw-cli chat` — interactive REPL
  - Connect WebSocket first (listen for responses)
  - Read user input line by line
  - POST each line as chat message
  - Display responses as they arrive on WS
  - Handle multi-line input (paste detection or explicit mode)
  - Ctrl+C for clean exit
  - History support (optional, via `rustyline`)

### Phase 4 — Admin Commands

- [ ] `gw-cli credentials list` — list all credentials (JSON table)
- [ ] `gw-cli credentials create` — create credential with all fields
  - `--route` accepts JSON string
  - `--config` accepts JSON string (optional)
  - `--backend` names which backend to route to
- [ ] `gw-cli credentials activate <id>` — activate credential
- [ ] `gw-cli credentials deactivate <id>` — deactivate credential

### Phase 5 — Polish

- [ ] CI integration (build gw-cli in existing workflow)
- [ ] Man page / `--help` examples
- [ ] Shell completion generation (clap feature)
- [ ] Config file support (optional: `~/.config/gw/config.toml`)

## Usage Examples

### Testing Pipelit Integration

```bash
# Set up environment
export GATEWAY_URL=http://localhost:8080
export GATEWAY_TOKEN=my-credential-token
export GATEWAY_ADMIN_TOKEN=my-admin-token

# Check gateway is running
gw-cli health

# Interactive chat with a Pipelit workflow
gw-cli chat my_chat --chat-id test-session-1

# One-shot send (useful in scripts)
gw-cli send my_chat --chat-id test-1 --text "Run the analysis workflow"

# Listen for responses (pipe to jq for pretty printing)
gw-cli listen my_chat --chat-id test-1 | jq .

# Pipe input from file
cat prompt.txt | gw-cli send my_chat --chat-id test-1
```

### Agent Usage (AI agent calling gw as a tool)

```bash
# Send and capture message ID
RESULT=$(gw-cli send my_chat --chat-id session-1 --text "analyze this data" --json)
echo $RESULT  # {"message_id":"generic_xxx","timestamp":"..."}

# Stream responses as JSONL (agent reads line by line)
gw-cli listen my_chat --chat-id session-1 --json
# {"text":"Processing...","timestamp":"...","message_id":"...","file_urls":[]}
# {"text":"Analysis complete.","timestamp":"...","message_id":"...","file_urls":["http://..."]}
```

### Admin Scripting

```bash
# Create a credential for a new workflow
gw-cli credentials create prod_workflow \
  --adapter generic \
  --backend pipelit \
  --token "$(openssl rand -hex 32)" \
  --route '{"workflow_slug":"production-pipeline","trigger_node_id":"node_entry"}'

# Activate it
gw-cli credentials activate prod_workflow

# List all credentials
gw-cli credentials list | jq '.[] | select(.active == true)'
```

## Non-Goals

- **No TUI framework** (no ratatui/crossterm) — this is a CLI tool, not a TUI app. TUIs call it as subprocess.
- **No built-in retry/queue** — fire-and-forget. The gateway handles reliability.
- **No file upload** — v1 is text-only. File support can be added later via `--file` flag.
- **No multi-credential chat** — one credential per session. Use multiple terminal tabs.

## Success Criteria

1. `gw-cli send` + `gw-cli listen` can round-trip a message through gateway → Pipelit → gateway
2. `gw-cli chat` provides interactive experience comparable to a chat client
3. `echo "hello" | gw-cli send ... | jq .` works end-to-end (pipe-friendly)
4. `gw-cli listen ... | while read line; do ... done` works for agent consumption
5. All commands exit 0 on success, 1 on failure, with errors on stderr
