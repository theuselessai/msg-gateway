# Decisions — opencode-backend

## [2026-03-08] Design Decisions (from user interview)
- Session scope: Per chat_id (isolated) — each conversation gets its own OpenCode session
- Response mode: Synchronous (POST /session/:id/message) — wait for full response
- Auth format: token field = "username:password", split at first colon, encode as HTTP Basic Auth
- Model config: From credential.config.model = { "providerID": "...", "modelID": "..." }
- Request timeout: 120 seconds
- Error relay: NO — errors logged only, not sent back to users
- Session cleanup: Deferred — no TTL/eviction for v1
- Multi-part response: Join text parts with "\n\n"
- Concurrent session creation: Atomic check-then-insert under write lock
