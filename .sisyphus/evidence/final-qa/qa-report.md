# QA Report — feat/opencode-backend

**Date:** 2026-03-09  
**Branch:** feat/opencode-backend  
**Worktree:** /home/aka/programs/msg-gateway-opencode  
**PR:** #32 (OPEN) — feat(backend): implement OpenCode adapter

---

## Scenario Results

| # | Scenario | Expected | Actual | Status |
|---|----------|----------|--------|--------|
| 1 | Stub removed | No "not yet implemented" matches | 0 matches | ✅ PASS |
| 2 | Token validation test | test passes, error contains "username:password" | 1/1 passed | ✅ PASS |
| 3 | Model config validation test | test passes, error contains "model" | 1/1 passed | ✅ PASS |
| 4 | All unit tests | 12 tests pass | 12/12 passed | ✅ PASS |
| 5 | All integration tests | 30 tests pass | 30/30 passed | ✅ PASS |
| 6 | Full roundtrip integration test | test passes | 1/1 passed | ✅ PASS |
| 7 | Session reuse integration test | test passes, 1 session for 2 messages | 1/1 passed | ✅ PASS |
| 8 | E2E OpenCode scenarios | 2 scenarios pass | 2/2 passed (after fix) | ✅ PASS |
| 9 | PR exists | PR #32 OPEN | OPEN | ✅ PASS |

---

## Issues Found & Fixed

### Issue 1: E2E Session Reuse Race Condition
**Root cause:** Two concurrent messages for the same chat_id both created sessions because:
1. The `OPENCODE_SESSIONS` static used `RwLock` — both requests acquired read locks simultaneously, saw no session, and both proceeded to create one.
2. The release binary was stale (pre-fix).

**Fix applied:**
- `src/backend.rs`: Changed `RwLock<HashMap<String, String>>` to `tokio::sync::Mutex<HashMap<String, String>>` and hold the lock during the HTTP session creation call. This serializes concurrent session creation for the same chat_id.
- `e2e/features/step_definitions/opencode.steps.ts`: Added polling wait (up to 10s) to the session count assertion step, since messages are processed asynchronously after the 202 response.
- Rebuilt release binary (`cargo build --release`) so E2E tests use the fixed code.

**Commit:** `81a71a4` — pushed to `feat/opencode-backend`

---

## Final Counts

- **Scenarios:** 9/9 pass
- **Unit tests:** 12/12
- **Integration tests:** 30/30
- **E2E scenarios:** 2/2
- **Edge cases tested:** Token format, missing model config, concurrent session creation, session reuse, auth relay, error responses

---

## VERDICT: APPROVE (with fix applied)
