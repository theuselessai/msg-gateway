# tests/ — Integration & WebSocket Tests

## FILES

| File | Lines | Purpose |
|------|-------|---------|
| `integration_test.rs` | ~901 | 30+ test cases: config, auth, send, inbound, files, admin CRUD |
| `ws_test.rs` | ~88 | WebSocket test; `#[ignore]` — requires running server |
| `mock_pipelit.py` | ~50 | Python mock backend for Pipelit webhook testing |

## CRITICAL PATTERNS

- **Env var tests MUST use `#[serial]`** — `serial_test` crate prevents race conditions on global state
- **`unsafe { std::env::set_var(...) }`** — Required in Rust 2024 edition; every set_var needs unsafe block
- **`#[tokio::test]`** — All async tests use tokio runtime
- **Dynamic port allocation** — `TcpListener::bind("127.0.0.1:0")` finds free ports; no hardcoded ports

## TEST HELPERS (integration_test.rs)

- `test_config()` — Creates minimal gateway config with dynamic port
- `test_config_with_file_cache()` — Config with file cache enabled + temp dir
- `find_available_port()` — Returns unused port via bind-to-zero
- `TestServer` struct — Spawns real Axum server; `Drop` impl aborts on cleanup

## RUNNING TESTS

```
cargo test                                     # All tests
cargo test --test integration_test             # Integration only
cargo test --test ws_test -- --ignored         # WS test (start server first!)
cargo llvm-cov                                 # Coverage report
```

## GOTCHAS

- WS test is `#[ignore]` by default — run manually with `--ignored --nocapture`
- WS test needs: `GATEWAY_CONFIG=config.example.json cargo run` in separate terminal
- Integration tests spawn real servers — avoid parallel execution of env-var tests
- `mock_pipelit.py` is a simple Flask app; not auto-started by tests
- All 14 source files also have inline `#[cfg(test)] mod tests {}` — unit tests live alongside code
