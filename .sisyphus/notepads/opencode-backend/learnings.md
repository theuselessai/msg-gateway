# Learnings — opencode-backend

## [2026-03-08] Session Start
- Worktree: /home/aka/programs/msg-gateway-opencode (branch: feat/opencode-backend)
- Plan: 6 implementation tasks + 4 final verification tasks
- Critical architecture: `create_adapter` called fresh per request at 4 sites — must use static OnceLock for session map
- OnceLock pattern is at `src/adapter.rs:17` (NOT line 175 as originally noted)
- Existing mock servers in e2e/ use Node's `http.createServer()`, NOT Fastify
- `gateway_url` computed as `format!("http://{}", listen_addr)` at `server.rs:51`
- `send_token` lives in `config.auth.send_token`
- `credential.config` (Option<serde_json::Value>) holds model config for OpenCode

## [2026-03-09] Task 1 Complete — create_adapter signature change
- Added `GatewayContext` struct and `OPENCODE_SESSIONS` static OnceLock to `backend.rs`
- Extended `create_adapter`, `PipelitAdapter::new`, and `OpencodeAdapter::new` signatures with `gateway_ctx` and `credential_config` params
- PipelitAdapter ignores both new params (prefixed with `_`)
- OpencodeAdapter stores them (gateway_url, send_token default to empty string if None)
- Updated 4 call sites: server.rs (adapter_inbound), generic.rs (chat_inbound), health.rs (drain_buffered_messages x2)
- health.rs default_target path passes `None` for credential_config (no credential available)
- All 220 unit tests + 25 integration tests pass, clippy clean
- `get_opencode_sessions()` helper clones the Arc for each adapter instance — lightweight

## [2026-03-09] Task 2 Complete — send_message() implementation
- No `base64` crate in Cargo.toml — used reqwest's `.basic_auth(username, Some(password))` instead
- Token split at first `:` (not last) to support passwords containing colons
- Session creation uses read-lock-first, then write-lock with double-check pattern for thread safety
- New reqwest client per `send_message()` call with 120s timeout (self.client has no timeout configured)
- Self-relay endpoint: POST `/api/v1/send` expects `credential_id`, `chat_id`, `text` as JSON fields
- Self-relay auth: `Authorization: Bearer {send_token}` header
- Self-relay failure (network or non-2xx) logged but not propagated — returns Ok(())
- OpenCode API: session response has `"id"` field; message response has `"parts"` array with `"type"` and `"text"` fields
- Removed `#[allow(dead_code)]` from 6 fields; kept on `client` and `poll_interval_ms`
- All 220 unit + 25 integration tests still pass, clippy clean

## [2026-03-09] Task 3 Complete — unit tests for OpencodeAdapter
- Added 8 new unit tests to `#[cfg(test)] mod tests` in `backend.rs` (total now 12 backend tests)
- Helper `make_opencode_target(token)` reduces boilerplate for TargetConfig construction
- Helper `make_dummy_message()` builds minimal InboundMessage for async send_message() tests
- `InboundMessage` requires: route (Value), credential_id, source (MessageSource), text, attachments (Vec), timestamp, extra_data (Option)
- `MessageSource` requires: protocol, chat_id, message_id, reply_to_message_id (Option), from (UserInfo)
- `UserInfo` requires: id, username (Option), display_name (Option)
- Async tests use `#[tokio::test]` — validation errors (token/model) fire BEFORE any HTTP calls, so no mocking needed
- `send_message()` validates token format first, then model config — both return `BackendError::InvalidConfig`
- Token with no colon → error contains "username:password"; missing model → error contains "model"
- `supports_files()` returns `false` for OpencodeAdapter (confirmed by test)
- All 228 unit + 25 integration tests pass, clippy clean

## [2026-03-09] Task 4 Complete — Integration tests for OpenCode backend

- Added `spawn_mock_opencode(error_on_message: bool)` — Axum server with POST /session and POST /session/{id}/message
- Uses `Arc<std::sync::Mutex<Vec<MockCapturedRequest>>>` for capturing requests (path, headers, body)
- `test_config_with_opencode_backend()` creates Config with "test_opencode" credential: adapter=generic, target=Opencode
- Token format "testuser:testpass" → reqwest `.basic_auth()` produces `Basic dGVzdHVzZXI6dGVzdHBhc3M=`
- 5 new tests all pass: full_roundtrip (WS), session_reuse, auth_basic, model_config_sent, error_response
- All 30 integration tests pass (25 existing + 5 new), clippy clean
- Key pattern: Axum closures as handlers need `Arc<Mutex<...>>` (Clone) for state capture — `move || { let cap = arc.clone(); async move { ... } }`
- OPENCODE_SESSIONS is static — used unique chat_ids per test (oc-rt-chat, oc-reuse-chat, etc.) to avoid interference
- Session reuse test uses 1000ms sleep between messages to ensure first background task completes before second starts
- Error test: mock returns 500 for /session/{id}/message — gateway still returns 202 (fire-and-forget) and stays alive
- `String` body extractor in Axum handles both empty body (POST /session) and JSON body gracefully

## [2026-03-09] Task 5 Complete — E2E Gherkin scenarios for OpenCode backend

### Files created/modified
- `e2e/support/mock-opencode-server.ts` — Node http.createServer() mock; implements POST /session, POST /session/:id/message, GET /global/health
- `e2e/features/opencode.feature` — 2 @opencode scenarios
- `e2e/features/step_definitions/opencode.steps.ts` — step implementations
- `e2e/features/step_definitions/world.ts` — added mockOpencodeServer property + After hook cleanup
- `e2e/support/test-gateway.ts` — added startWithOpencodeConfig(opencodePort) method

### Key patterns
- Mock server tracks `allMessages` (never shifted) separately from `pendingMessages` (shifted by waiters) — critical for `getMessages()` count assertions after `waitForMessage()` calls
- `startWithOpencodeConfig` uses per-credential `target` override (not default_target) so the generic adapter uses OpenCode backend
- Credential config: `config: { model: { providerID, modelID } }` required by OpencodeAdapter.send_message()
- Token format: `"testuser:testpass"` (username:password split at first colon)
- WS URL: `ws://127.0.0.1:{port}/ws/chat/test_opencode/{chatId}` with `Authorization: Bearer generic_token`
- Self-relay flow: gateway → mock OpenCode → gateway `/api/v1/send` → WS client
- Telegram E2E tests are pre-existing failures (Python adapter subprocess timeout) — not related to OpenCode work
- `npx cucumber-js` resolves to wrong package; use `./node_modules/.bin/cucumber-js` directly
- `--tags '@opencode'` exits 0 with 2 passing scenarios
