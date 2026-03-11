# CEL Guardrails + XDG Config Directory

## TL;DR

> **Quick Summary**: Add a CEL expression-based guardrail system for filtering inbound messages, with rules as individual JSON files in a `guardrails/` directory. Also restructure config path resolution to follow XDG conventions with auto-discovery.
> 
> **Deliverables**:
> - `src/guardrail.rs` — New guardrail engine module with CEL compilation, evaluation, file loading
> - XDG-compliant config path resolution (`~/.config/msg-gateway/`)
> - `guardrails_dir` auto-discovery and hot-reload
> - Integration into inbound message handlers (adapter + generic)
> - TDD test suite covering all guardrail scenarios
> 
> **Estimated Effort**: Medium (2-3d)
> **Parallel Execution**: YES — 3 waves
> **Critical Path**: Task 1 → Task 4 → Task 6 → Task 8 → Task 10 → Task 11 → F1-F4

---

## Context

### Original Request
Add middleware-like capabilities to msg-gateway for filtering messages. After evaluating redux-rs, Lua, Rhai, JS/WASM, and CEL, decided on CEL as rules-as-config approach. Rules live in individual JSON files in a `guardrails/` directory alongside the config file, following XDG conventions.

### Interview Summary
**Key Discussions**:
- Redux-rs rejected: designed for UI state management, not message pipelines (blacktrade is TUI, msg-gateway is async gateway)
- Lua/Rhai/JS rejected: CEL simpler for filter-only rules, no runtime needed, single config file model
- CEL chosen: expression-only, no side effects, rules = JSON files, hot-reloadable
- Rules as individual files: ordered by filename prefix, auto-discovered in `guardrails/` directory
- XDG config dir: `~/.config/msg-gateway/` default, `GATEWAY_CONFIG` env override
- Outbound guardrails deferred to v2 (different context shape, trusted backend source)

**Research Findings**:
- `cel-interpreter` 0.8: `Program` is `Send+Sync` but NOT `Clone` → use `Arc<Program>`
- `has()` macro NOT supported → must pre-convert `Option<T>` to `Value::Null`
- `matches()` uses Rust regex crate, NOT RE2 — different syntax from Google CEL spec
- `Context` not reusable — fresh per evaluation
- `serde_json::Value` → CEL `Value` needs manual converter (no built-in `From`)
- msg-gateway already uses `notify` crate for file watching with debounce

### Metis Review
**Identified Gaps** (addressed):
- **`has()` not available**: Pre-convert all Option fields to null in CEL context builder; document limitation
- **`Program` not Clone**: Use `Arc<Program>` in CompiledRule
- **`matches()` uses Rust regex, not RE2**: Document explicitly, use Rust regex syntax in examples
- **XDG path change breaks existing adapters_dir**: Only `guardrails_dir` uses config-relative resolution; keep CWD for existing paths
- **Hot-reload race conditions**: Debounce guardrails directory events (1000ms, matching existing watcher pattern)
- **Outbound guardrails context mismatch**: Defer to v2 — inbound only for this plan
- **Rule ordering ambiguity**: Lexicographic sort, recommend zero-padded prefixes in docs
- **Buffered message re-evaluation**: Skip — drain replays without re-evaluation

---

## Work Objectives

### Core Objective
Add a CEL-based guardrail system that filters inbound messages using rules defined as individual JSON files, with XDG-compliant config path resolution and hot-reload support.

### Concrete Deliverables
- `src/guardrail.rs`: GuardrailEngine with compiled CEL program cache, file-per-rule loading, evaluation pipeline
- `src/config.rs`: GuardrailConfig types, `guardrails_dir` field with auto-discovery
- `src/main.rs`: XDG config path resolution function
- Guardrail intercepts in `adapter_inbound()` and `chat_inbound()` handlers
- `AppError::Forbidden` variant for blocked messages
- Watcher monitors `guardrails/` directory for hot-reload
- Example guardrail rule files
- TDD test suite

### Definition of Done
- [ ] `cargo test` — all tests pass (existing + new guardrail tests)
- [ ] `cargo clippy --all-targets -- -D warnings` — zero warnings
- [ ] `cargo fmt --all -- --check` — formatted
- [ ] Inbound message matching a `block` rule returns HTTP 403
- [ ] Inbound message not matching any rule returns HTTP 202
- [ ] Invalid CEL expression in rule file is logged and skipped (gateway starts)
- [ ] Guardrail rules hot-reload when files change in `guardrails/` directory
- [ ] Config resolves from `GATEWAY_CONFIG` → XDG → CWD fallback

### Must Have
- CEL expression evaluation with compiled program cache (compile once per config load)
- Short-circuit: first `block` rule stops pipeline
- Fail-open default: CEL errors → allow message (per-rule configurable)
- Individual JSON rule files, lexicographic filename ordering
- Auto-discovery of `guardrails/` directory alongside config file
- XDG config path resolution with backward-compatible `GATEWAY_CONFIG` env override
- `AppError::Forbidden` with HTTP 403 status
- Hot-reload when guardrail files change

### Must NOT Have (Guardrails)
- Outbound guardrails (send_message path) — deferred to v2
- Per-credential guardrail overrides — all rules apply globally
- Admin API endpoints for guardrail CRUD — files-on-disk is the interface
- LLM guardrail implementation (schema placeholder `type` field only)
- Message transformation — CEL is filter-only (allow/block/log)
- Guardrails on WebSocket handler (outbound only)
- Re-evaluation of buffered messages on drain
- Modification of `InboundMessage` struct (guardrails consume, not annotate)
- Migration of `adapters_dir`/`backends_dir` to config-relative resolution (keep CWD for backward compat)
- Over-abstracted middleware pipeline trait — keep it simple, direct evaluation in handlers

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed. No exceptions.

### Test Decision
- **Infrastructure exists**: YES (`cargo test`, `#[tokio::test]`, `cargo llvm-cov`)
- **Automated tests**: TDD (RED → GREEN → REFACTOR)
- **Framework**: cargo test + tokio::test
- **If TDD**: Each task writes failing test first → minimal impl → verify pass → refactor

### QA Policy
Every task MUST include agent-executed QA scenarios.
Evidence saved to `.sisyphus/evidence/task-{N}-{scenario-slug}.{ext}`.

- **Unit tests**: `cargo test` — CEL evaluation, file loading, config parsing
- **Integration tests**: TestServer pattern + curl — blocked message returns 403, allowed returns 202
- **Config tests**: Environment variable manipulation with `#[serial]`

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Foundation — independent, start immediately):
├── Task 1: Add cel-interpreter dependency + AppError::Forbidden variant [quick]
├── Task 2: json_to_cel_value() converter + unit tests [quick]
├── Task 3: GuardrailRule config types + serde + unit tests [quick]
└── Task 4: XDG config path resolution function + unit tests [quick]

Wave 2 (Core engine — depends on Wave 1):
├── Task 5: Rule file loading from directory + unit tests (depends: 3) [unspecified-high]
├── Task 6: GuardrailEngine compile + evaluate + unit tests (depends: 2, 3) [deep]
└── Task 7: guardrails_dir in GatewayConfig + resolve_relative_paths (depends: 3, 4) [quick]

Wave 3 (Integration — depends on Wave 2):
├── Task 8: Wire GuardrailEngine into AppState + intercept handlers (depends: 6, 7) [deep]
├── Task 9: Watcher: monitor guardrails/ directory + rebuild engine (depends: 7, 8) [unspecified-high]
└── Task 10: Integration tests: blocked/allowed messages end-to-end (depends: 8) [deep]

Wave 4 (Polish):
└── Task 11: Example rule files + config.example.json update + README (depends: 8) [writing]

Wave FINAL (After ALL tasks — independent review, 4 parallel):
├── Task F1: Plan compliance audit (oracle)
├── Task F2: Code quality review (unspecified-high)
├── Task F3: Real manual QA (unspecified-high)
└── Task F4: Scope fidelity check (deep)

Critical Path: Task 1 → Task 6 → Task 8 → Task 10 → F1-F4
Parallel Speedup: ~55% faster than sequential
Max Concurrent: 4 (Wave 1)
```

### Dependency Matrix

| Task | Depends On | Blocks | Wave |
|------|-----------|--------|------|
| 1 | — | 6, 8 | 1 |
| 2 | — | 6 | 1 |
| 3 | — | 5, 6, 7 | 1 |
| 4 | — | 7 | 1 |
| 5 | 3 | 6, 9 | 2 |
| 6 | 1, 2, 3, 5 | 8, 10 | 2 |
| 7 | 3, 4 | 8, 9 | 2 |
| 8 | 6, 7 | 9, 10, 11 | 3 |
| 9 | 7, 8 | — | 3 |
| 10 | 8 | — | 3 |
| 11 | 8 | — | 4 |

### Agent Dispatch Summary

- **Wave 1**: 4 tasks — T1 → `quick`, T2 → `quick`, T3 → `quick`, T4 → `quick`
- **Wave 2**: 3 tasks — T5 → `unspecified-high`, T6 → `deep`, T7 → `quick`
- **Wave 3**: 3 tasks — T8 → `deep`, T9 → `unspecified-high`, T10 → `deep`
- **Wave 4**: 1 task — T11 → `writing`
- **FINAL**: 4 tasks — F1 → `oracle`, F2 → `unspecified-high`, F3 → `unspecified-high`, F4 → `deep`

---

## TODOs

- [x] 1. Add cel-interpreter dependency + AppError::Forbidden variant

  **What to do**:
  - Add `cel-interpreter = "0.8"` to `Cargo.toml` `[dependencies]`
  - Add `Forbidden(String)` variant to `AppError` enum in `src/error.rs`
  - Add `IntoResponse` mapping: `AppError::Forbidden(msg)` → `StatusCode::FORBIDDEN` (403) with JSON error body
  - TDD: Write test for `Forbidden` variant → 403 status + correct body → implement → verify pass
  - Run `cargo build` to verify cel-interpreter compiles with project

  **Must NOT do**:
  - Do not add any guardrail logic yet
  - Do not modify any handler

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Single dependency add + one enum variant, minimal code
  - **Skills**: []
  - **Skills Evaluated but Omitted**:
    - `playwright`: No browser interaction
    - `git-master`: Simple commit, no complex git ops

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 2, 3, 4)
  - **Blocks**: Tasks 6, 8
  - **Blocked By**: None (can start immediately)

  **References**:

  **Pattern References**:
  - `src/error.rs:8-39` — Existing `AppError` enum with `IntoResponse` impl. Follow exact same pattern for new variant. Note the match arm structure mapping variant → (StatusCode, message).
  - `src/error.rs:41-72` — `IntoResponse` implementation with JSON body format `{"error": "message"}`. New variant must follow identical JSON structure.

  **API/Type References**:
  - `Cargo.toml` — Current dependencies section. Add `cel-interpreter` alongside existing deps.

  **External References**:
  - `cel-interpreter` crate: https://crates.io/crates/cel-interpreter — verify version 0.8 exists and compiles

  **Acceptance Criteria**:

  **TDD:**
  - [ ] Test written FIRST: `AppError::Forbidden("blocked".into())` produces HTTP 403 with body `{"error":"blocked"}`
  - [ ] `cargo build` succeeds (cel-interpreter compiles)
  - [ ] `cargo test` passes (existing + new test)
  - [ ] `cargo clippy --all-targets -- -D warnings` passes

  **QA Scenarios:**

  ```
  Scenario: Forbidden error produces correct HTTP response
    Tool: Bash (cargo test)
    Preconditions: New test exists in error.rs
    Steps:
      1. Run `cargo test test_forbidden_error -- --exact`
      2. Verify test asserts: status == 403, body contains "error" key, body contains exact message string
    Expected Result: Test passes with 0 failures
    Failure Indicators: Compilation error, wrong status code, missing JSON body
    Evidence: .sisyphus/evidence/task-1-forbidden-error-test.txt

  Scenario: cel-interpreter compiles successfully
    Tool: Bash (cargo build)
    Preconditions: cel-interpreter added to Cargo.toml
    Steps:
      1. Run `cargo build 2>&1`
      2. Verify exit code 0
      3. Grep output for "Compiling cel-interpreter"
    Expected Result: Build succeeds, cel-interpreter downloaded and compiled
    Failure Indicators: Compilation error, version not found, feature flag issue
    Evidence: .sisyphus/evidence/task-1-cel-interpreter-build.txt
  ```

  **Commit**: YES (group 1)
  - Message: `feat(deps): add cel-interpreter and AppError::Forbidden variant`
  - Files: `Cargo.toml`, `src/error.rs`
  - Pre-commit: `cargo build && cargo test`

- [x] 2. json_to_cel_value() converter + unit tests

  **What to do**:
  - Create `src/guardrail.rs` with a `json_to_cel_value(serde_json::Value) -> cel_interpreter::Value` function
  - This converter recursively maps: JSON null → CEL Null, string → String, number → Int/UInt/Float, bool → Bool, array → List, object → Map
  - Critical: `Option<T>` fields serialize to JSON `null` via serde → must map to CEL `Value::Null` (because `has()` is NOT available in cel-interpreter)
  - TDD: Write failing tests FIRST for each JSON type → implement converter → verify pass
  - Add `pub mod guardrail;` to `src/main.rs` and `src/lib.rs`

  **Must NOT do**:
  - Do not add GuardrailEngine or evaluation logic yet
  - Do not add config types (Task 3)
  - Do not use `has()` anywhere — it doesn't exist in cel-interpreter

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Single function with recursive pattern matching, well-defined inputs/outputs
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 3, 4)
  - **Blocks**: Task 6
  - **Blocked By**: None (can start immediately)

  **References**:

  **Pattern References**:
  - `src/message.rs` — `InboundMessage` struct with `#[derive(Serialize)]`. Shows which fields are `Option<T>` (these become JSON null → CEL Null).

  **API/Type References**:
  - `cel_interpreter::Value` — The target type. Variants include: `Value::Null`, `Value::Bool(bool)`, `Value::Int(i64)`, `Value::UInt(u64)`, `Value::Float(f64)`, `Value::String(Arc<String>)`, `Value::List(Arc<Vec<Value>>)`, `Value::Map(Map)`.
  - `serde_json::Value` — The source type. Variants: `Null`, `Bool`, `Number`, `String`, `Array`, `Object`.

  **External References**:
  - cel-interpreter source `value.rs`: https://github.com/clarkmcc/cel-rust — shows Value constructors and Map type

  **Acceptance Criteria**:

  **TDD:**
  - [ ] Tests written FIRST for: null, bool, string, integer, float, array, nested object, mixed nested, empty object, empty array
  - [ ] `json_to_cel_value(Value::Null)` → `Value::Null`
  - [ ] `json_to_cel_value(Value::String("hello"))` → `Value::String(Arc::new("hello".into()))`
  - [ ] `json_to_cel_value(Value::Number(42))` → `Value::Int(42)` (or UInt for positive)
  - [ ] Nested: `{"a": {"b": null}}` → Map with "a" → Map with "b" → Null
  - [ ] `cargo test guardrail::tests` passes

  **QA Scenarios:**

  ```
  Scenario: Converter handles all JSON types correctly
    Tool: Bash (cargo test)
    Preconditions: guardrail.rs exists with converter + tests
    Steps:
      1. Run `cargo test guardrail::tests::test_json_to_cel -- --nocapture`
      2. Verify all type conversion tests pass
    Expected Result: All tests pass, including null/nested/empty edge cases
    Failure Indicators: Type mismatch, panic on null, incorrect Map key format
    Evidence: .sisyphus/evidence/task-2-converter-tests.txt

  Scenario: Option<T> fields convert to CEL Null (not error)
    Tool: Bash (cargo test)
    Preconditions: Test creates InboundMessage with None fields, serializes, converts
    Steps:
      1. Run `cargo test guardrail::tests::test_option_fields_to_null -- --nocapture`
      2. Verify accessing a None field in CEL context produces Null, not NoSuchKey error
    Expected Result: Null value accessible without error
    Failure Indicators: NoSuchKey error, panic, test failure
    Evidence: .sisyphus/evidence/task-2-option-null-test.txt
  ```

  **Commit**: YES (group 2 — with Task 3)
  - Message: `feat(guardrail): CEL value converter and rule config types with TDD tests`
  - Files: `src/guardrail.rs`, `src/main.rs`, `src/lib.rs`
  - Pre-commit: `cargo test`

- [x] 3. GuardrailRule config types + serde + unit tests

  **What to do**:
  - Add to `src/config.rs`: `GuardrailRule`, `GuardrailAction` (Block/Log), `GuardrailDirection` (Inbound/Outbound/Both), `GuardrailOnError` (Allow/Block), `GuardrailType` (Cel) — all with serde derive + defaults
  - `GuardrailRule` fields: `name: String`, `r#type: GuardrailType` (default Cel), `expression: String`, `action: GuardrailAction` (default Block), `direction: GuardrailDirection` (default Inbound), `on_error: GuardrailOnError` (default Allow), `reject_message: Option<String>`, `enabled: bool` (default true)
  - TDD: Write serde deserialization tests FIRST (minimal JSON → full struct with defaults, full JSON → all fields populated) → implement types → verify pass

  **Must NOT do**:
  - Do not add `guardrails_dir` to `GatewayConfig` yet (Task 7)
  - Do not add file loading logic (Task 5)
  - Do not add `GuardrailConfig` — rules are loaded from files, not config.json

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Pure data types with serde derives, no logic
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2, 4)
  - **Blocks**: Tasks 5, 6, 7
  - **Blocked By**: None (can start immediately)

  **References**:

  **Pattern References**:
  - `src/config.rs:56-65` — `BackendProtocol` enum with `#[serde(rename_all = "lowercase")]`. Follow this exact pattern for `GuardrailAction`, `GuardrailDirection`, `GuardrailOnError`, `GuardrailType`.
  - `src/config.rs:39-53` — Default value functions pattern (`fn default_adapters_dir()`). Use same pattern for `fn default_true() -> bool { true }`.
  - `src/config.rs:68-99` — `TargetConfig` struct with optional fields and `#[serde(default)]`. Follow for `GuardrailRule`.

  **Acceptance Criteria**:

  **TDD:**
  - [ ] Test: `{"name":"test","expression":"true"}` deserializes with all defaults (type=cel, action=block, direction=inbound, on_error=allow, enabled=true)
  - [ ] Test: Full JSON with all fields populated deserializes correctly
  - [ ] Test: `enabled: false` deserializes as false
  - [ ] Test: Serialize → Deserialize roundtrip preserves all fields
  - [ ] `cargo test config::tests` passes (new + existing)

  **QA Scenarios:**

  ```
  Scenario: Minimal rule JSON deserializes with correct defaults
    Tool: Bash (cargo test)
    Steps:
      1. Run `cargo test config::tests::test_guardrail_rule_defaults -- --exact`
      2. Verify: type == Cel, action == Block, direction == Inbound, on_error == Allow, enabled == true
    Expected Result: All defaults correct
    Evidence: .sisyphus/evidence/task-3-rule-defaults-test.txt

  Scenario: Invalid action value rejected by serde
    Tool: Bash (cargo test)
    Steps:
      1. Run `cargo test config::tests::test_guardrail_rule_invalid_action -- --exact`
      2. Verify: deserialization returns Err for `"action": "invalid_value"`
    Expected Result: serde error, not panic
    Evidence: .sisyphus/evidence/task-3-rule-invalid-test.txt
  ```

  **Commit**: YES (group 2 — with Task 2)
  - Message: `feat(guardrail): CEL value converter and rule config types with TDD tests`
  - Files: `src/config.rs`
  - Pre-commit: `cargo test`

- [x] 4. XDG config path resolution function + unit tests

  **What to do**:
  - Add a `pub fn resolve_config_path() -> PathBuf` function in `src/config.rs`
  - Resolution order: (1) `GATEWAY_CONFIG` env → use as-is, (2) `$XDG_CONFIG_HOME/msg-gateway/config.json` or `$HOME/.config/msg-gateway/config.json` if exists, (3) `./config.json` fallback
  - Use only `std::env` and `std::path` — no external crates (`dirs`)
  - Update `main.rs` to call `resolve_config_path()` instead of inline env var logic
  - TDD: Write tests with `#[serial]` (since env var manipulation) → implement → verify
  - IMPORTANT: Tests that modify env vars MUST use `#[serial]` attribute from `serial_test` crate (already in dev-dependencies — verify)

  **Must NOT do**:
  - Do not change `adapters_dir`/`backends_dir` resolution (keep CWD-relative for backward compat)
  - Do not add `guardrails_dir` yet (Task 7)
  - Do not restructure existing config loading (`load_config` stays the same)

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Single function, pure path logic, well-defined behavior
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2, 3)
  - **Blocks**: Task 7
  - **Blocked By**: None (can start immediately)

  **References**:

  **Pattern References**:
  - `src/main.rs:35` — Current config path logic: `std::env::var("GATEWAY_CONFIG").unwrap_or_else(|_| "config.json".to_string())`. This is what we're replacing.
  - `src/config.rs:137-148` — `load_config()` function. It takes `&str` path. The new `resolve_config_path()` returns `PathBuf` to feed into `load_config()`.

  **External References**:
  - XDG Base Directory Specification: `$XDG_CONFIG_HOME` defaults to `$HOME/.config`

  **Acceptance Criteria**:

  **TDD:**
  - [ ] Test: `GATEWAY_CONFIG=/tmp/custom.json` → returns `/tmp/custom.json` (regardless of file existence)
  - [ ] Test: No `GATEWAY_CONFIG` + `XDG_CONFIG_HOME=/tmp/xdg` + file exists at `/tmp/xdg/msg-gateway/config.json` → returns that path
  - [ ] Test: No env vars set + no XDG file → returns `./config.json` (CWD fallback)
  - [ ] Test: `HOME` set + no `XDG_CONFIG_HOME` + file exists at `$HOME/.config/msg-gateway/config.json` → returns that path
  - [ ] All tests use `#[serial]` for env var safety
  - [ ] `cargo test config::tests::test_resolve_config_path` passes

  **QA Scenarios:**

  ```
  Scenario: GATEWAY_CONFIG env override takes priority
    Tool: Bash (cargo test)
    Preconditions: Tests use #[serial] and temp directories
    Steps:
      1. Run `cargo test config::tests::test_resolve_config_path_env_override -- --exact`
      2. Verify: returns exact path from GATEWAY_CONFIG, doesn't check XDG
    Expected Result: Test passes
    Evidence: .sisyphus/evidence/task-4-xdg-env-override.txt

  Scenario: XDG fallback when no env var set
    Tool: Bash (cargo test)
    Steps:
      1. Run `cargo test config::tests::test_resolve_config_path_xdg_fallback -- --exact`
      2. Verify: creates temp dir, places config file, resolves correctly
    Expected Result: Test passes, correct path returned
    Evidence: .sisyphus/evidence/task-4-xdg-fallback.txt
  ```

  **Commit**: YES (group 3)
  - Message: `feat(config): XDG-compliant config path resolution`
  - Files: `src/config.rs`, `src/main.rs`
  - Pre-commit: `cargo test`

- [x] 5. Rule file loading from directory + unit tests

  **What to do**:
  - Add `pub fn load_rules_from_dir(dir: &Path) -> Vec<GuardrailRule>` in `src/guardrail.rs`
  - Reads all `*.json` files in directory, sorts by filename (lexicographic), deserializes each as `GuardrailRule`
  - Skips files that fail to parse (log error, continue with remaining)
  - Skips files with `enabled: false` (log debug, continue)
  - Returns empty vec if directory doesn't exist or is empty
  - TDD: Write tests using `tempdir` with fixture rule files → implement → verify

  **Must NOT do**:
  - Do not compile CEL expressions here (Task 6 does that)
  - Do not add hot-reload / watcher logic (Task 9)
  - Do not skip `.json.disabled` files by convention — use `enabled: false` field instead

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: File I/O + error handling + sorting + TDD, moderate complexity
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 6, 7)
  - **Blocks**: Task 6
  - **Blocked By**: Task 3 (needs GuardrailRule type)

  **References**:

  **Pattern References**:
  - `src/adapter.rs:155-210` — `AdapterInstanceManager::new()` reads adapter directories, discovers `adapter.json` files. Similar pattern: read dir → filter files → parse JSON → collect. Follow error handling style.

  **API/Type References**:
  - `src/config.rs` — `GuardrailRule` struct (from Task 3). Deserialize target.

  **Acceptance Criteria**:

  **TDD:**
  - [ ] Test: Dir with 3 valid rule files → returns 3 rules in filename order
  - [ ] Test: Dir with `02_b.json` and `01_a.json` → returns `[a, b]` (sorted)
  - [ ] Test: Dir with 1 valid + 1 malformed JSON → returns 1 rule + logs error
  - [ ] Test: Dir with 1 valid + 1 `enabled: false` → returns 1 rule (enabled only)
  - [ ] Test: Non-existent dir → returns empty vec (no error)
  - [ ] Test: Empty dir → returns empty vec
  - [ ] Test: Dir with `.txt` and `.json` files → only `.json` files loaded
  - [ ] `cargo test guardrail::tests::test_load_rules` passes

  **QA Scenarios:**

  ```
  Scenario: Rules loaded in filename order
    Tool: Bash (cargo test)
    Steps:
      1. Run `cargo test guardrail::tests::test_load_rules_ordering -- --nocapture`
      2. Verify: rules returned in lexicographic filename order
    Expected Result: 01_ before 02_ before 10_
    Evidence: .sisyphus/evidence/task-5-rule-ordering.txt

  Scenario: Malformed JSON file skipped gracefully
    Tool: Bash (cargo test)
    Steps:
      1. Run `cargo test guardrail::tests::test_load_rules_malformed_skipped -- --nocapture`
      2. Verify: returns valid rules only, no panic, error logged
    Expected Result: 1 rule returned from 2 files (1 valid, 1 malformed)
    Evidence: .sisyphus/evidence/task-5-malformed-skip.txt
  ```

  **Commit**: YES (group 4 — with Task 6)
  - Message: `feat(guardrail): rule file loading and GuardrailEngine with CEL evaluation`
  - Files: `src/guardrail.rs`
  - Pre-commit: `cargo test`

- [x] 6. GuardrailEngine compile + evaluate + unit tests

  **What to do**:
  - Add `GuardrailEngine` struct in `src/guardrail.rs`: holds `Vec<CompiledRule>` where `CompiledRule` has `name: String`, `program: Arc<Program>`, `action: GuardrailAction`, `direction: GuardrailDirection`, `on_error: GuardrailOnError`, `reject_message: Option<String>`
  - `GuardrailEngine::from_rules(rules: Vec<GuardrailRule>) -> Self`: compiles each rule's CEL expression via `Program::compile()`. Invalid expressions logged and skipped.
  - `GuardrailEngine::evaluate_inbound(&self, message: &InboundMessage) -> GuardrailVerdict`: creates fresh `Context`, converts message to CEL value (using `json_to_cel_value` from Task 2), evaluates all inbound-applicable rules with short-circuit on block.
  - `GuardrailVerdict` enum: `Allow`, `Block { rule_name: String, reject_message: String }`
  - `GuardrailEngine::is_empty(&self) -> bool`: fast-path to skip evaluation when no rules
  - CRITICAL: `Program` is NOT Clone — must wrap in `Arc<Program>`
  - CRITICAL: `Context` must be created fresh per evaluation (not reusable)
  - CRITICAL: `has()` NOT available — pre-convert all Option fields to null via serde → json_to_cel_value
  - TDD: Write failing tests for each scenario → implement → verify

  **Must NOT do**:
  - Do not add outbound evaluation (v2)
  - Do not add to AppState (Task 8)
  - Do not add hot-reload (Task 9)
  - Do not use `has()` in any CEL expression or test

  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: Core business logic, CEL API integration, multiple edge cases, requires careful Arc/lifetime management
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 5, 7)
  - **Blocks**: Tasks 8, 10
  - **Blocked By**: Tasks 1 (cel-interpreter), 2 (converter), 3 (rule types), 5 (file loading)

  **References**:

  **Pattern References**:
  - `src/guardrail.rs` (from Tasks 2, 5) — `json_to_cel_value()` converter and `load_rules_from_dir()`. Use these directly.
  - `src/health.rs:40-80` — `HealthMonitor` struct pattern with `RwLock` fields. Similar encapsulation pattern.

  **API/Type References**:
  - `cel_interpreter::Program` — `Program::compile(&str) -> Result<Program, _>`. NOT Clone → must use `Arc<Program>`.
  - `cel_interpreter::Context` — `Context::default()`, `context.add_variable("name", value) -> Result<(), _>`. NOT reusable.
  - `cel_interpreter::Value` — Return type of `program.execute(&context)`. Match on `Value::Bool(true/false)`.
  - `src/message.rs` — `InboundMessage` struct with `#[derive(Serialize)]`. Serialize → serde_json::Value → json_to_cel_value.

  **External References**:
  - cel-interpreter threads example: https://github.com/clarkmcc/cel-rust — shows Arc<Program> usage across threads

  **Acceptance Criteria**:

  **TDD:**
  - [ ] Test: Rule `"true"` → `GuardrailVerdict::Allow`
  - [ ] Test: Rule `"false"` with action=block → `GuardrailVerdict::Block` with correct name + message
  - [ ] Test: Rule `"false"` with action=log → `GuardrailVerdict::Allow` (log only, not block)
  - [ ] Test: Two rules, first blocks → second never evaluated (short-circuit)
  - [ ] Test: Rule with invalid CEL → skipped during from_rules(), remaining rules work
  - [ ] Test: Rule with on_error=allow + runtime error → Allow
  - [ ] Test: Rule with on_error=block + runtime error → Block
  - [ ] Test: Rule with `enabled: false` → skipped (covered in load, but verify engine receives only enabled rules)
  - [ ] Test: `message.text.matches('password')` with matching text → Block
  - [ ] Test: `message.text.matches('password')` with non-matching text → Allow
  - [ ] Test: Access `message.source.from.username` when username is None → evaluates without panic (null handling)
  - [ ] Test: `is_empty()` returns true for empty engine, false otherwise
  - [ ] `cargo test guardrail::tests::test_engine` passes

  **QA Scenarios:**

  ```
  Scenario: CEL expression correctly evaluates against InboundMessage
    Tool: Bash (cargo test)
    Steps:
      1. Run `cargo test guardrail::tests::test_engine_evaluate_inbound -- --nocapture`
      2. Verify: message with "password" in text → Block; message with "hello" → Allow
    Expected Result: Both assertions pass
    Evidence: .sisyphus/evidence/task-6-engine-evaluate.txt

  Scenario: Short-circuit on first blocking rule
    Tool: Bash (cargo test)
    Steps:
      1. Run `cargo test guardrail::tests::test_engine_short_circuit -- --nocapture`
      2. Verify: second rule never evaluated when first blocks
    Expected Result: Block verdict from first rule, not second
    Evidence: .sisyphus/evidence/task-6-short-circuit.txt

  Scenario: Null field access doesn't panic
    Tool: Bash (cargo test)
    Steps:
      1. Run `cargo test guardrail::tests::test_engine_null_field_access -- --nocapture`
      2. Verify: accessing None field in CEL expression → on_error behavior, no panic
    Expected Result: Test passes with graceful handling
    Evidence: .sisyphus/evidence/task-6-null-field.txt
  ```

  **Commit**: YES (group 4 — with Task 5)
  - Message: `feat(guardrail): rule file loading and GuardrailEngine with CEL evaluation`
  - Files: `src/guardrail.rs`
  - Pre-commit: `cargo test`

- [x] 7. guardrails_dir in GatewayConfig + resolve_relative_paths

  **What to do**:
  - Add `guardrails_dir: Option<String>` to `GatewayConfig` in `src/config.rs` with `#[serde(default)]`
  - Add `fn resolve_guardrails_dir(config: &mut Config, config_dir: &Path)` in `src/config.rs`:
    - If `guardrails_dir` is `Some(path)` and path is relative → resolve relative to `config_dir`
    - If `guardrails_dir` is `None` → check if `{config_dir}/guardrails/` directory exists → set to that path
    - If `guardrails_dir` is `None` and directory doesn't exist → leave as None
  - Call `resolve_guardrails_dir()` from `load_config()` after parsing, using config file's parent dir
  - IMPORTANT: Do NOT change resolution of `adapters_dir` or `backends_dir` — keep CWD-relative for backward compat
  - TDD: Write tests with temp directories → implement → verify

  **Must NOT do**:
  - Do not change existing `adapters_dir`/`backends_dir`/`file_cache.directory` resolution
  - Do not add GuardrailEngine to AppState (Task 8)
  - Do not add watcher logic (Task 9)

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Single config field + path resolution function, straightforward
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 5, 6)
  - **Blocks**: Tasks 8, 9
  - **Blocked By**: Tasks 3 (config types), 4 (XDG resolution pattern)

  **References**:

  **Pattern References**:
  - `src/config.rs:24-37` — `GatewayConfig` struct with `adapters_dir` field and `#[serde(default = "...")]`. Follow same pattern for `guardrails_dir`.
  - `src/config.rs:137-148` — `load_config()` function. Add `resolve_guardrails_dir()` call after deserialization.

  **Acceptance Criteria**:

  **TDD:**
  - [ ] Test: `guardrails_dir: None` + `{config_dir}/guardrails/` exists → resolved to that dir
  - [ ] Test: `guardrails_dir: None` + no guardrails dir → remains None
  - [ ] Test: `guardrails_dir: "./my_rules"` + config at `/tmp/config.json` → resolves to `/tmp/my_rules`
  - [ ] Test: `guardrails_dir: "/absolute/path"` → stays as-is (absolute)
  - [ ] Test: Serde roundtrip with and without `guardrails_dir` field
  - [ ] Existing config tests still pass (backward compat)
  - [ ] `cargo test config::tests` passes

  **QA Scenarios:**

  ```
  Scenario: Auto-discovery of guardrails directory
    Tool: Bash (cargo test)
    Steps:
      1. Run `cargo test config::tests::test_guardrails_dir_auto_discovery -- --nocapture`
      2. Verify: creates temp config dir with guardrails/ subdir → resolves correctly
    Expected Result: guardrails_dir populated with correct absolute path
    Evidence: .sisyphus/evidence/task-7-auto-discovery.txt

  Scenario: Existing config without guardrails_dir loads fine
    Tool: Bash (cargo test)
    Steps:
      1. Run `cargo test config::tests` (all config tests)
      2. Verify: zero test failures (backward compat)
    Expected Result: All existing tests pass unchanged
    Evidence: .sisyphus/evidence/task-7-backward-compat.txt
  ```

  **Commit**: YES (group 5 — with Task 8)
  - Message: `feat(guardrail): wire engine into AppState and intercept inbound handlers`
  - Files: `src/config.rs`
  - Pre-commit: `cargo test`

- [x] 8. Wire GuardrailEngine into AppState + intercept inbound handlers

  **What to do**:
  - Add `pub guardrail_engine: RwLock<GuardrailEngine>` to `AppState` in `src/server.rs`
  - In `create_server()`: load rules from `guardrails_dir` (if Some) via `load_rules_from_dir()`, create `GuardrailEngine::from_rules()`, store in AppState
  - In `adapter_inbound()` handler (`src/server.rs`): after `InboundMessage` is constructed, before health check — acquire `guardrail_engine.read()`, call `evaluate_inbound()`, on `Block` return `AppError::Forbidden(reject_message)`
  - In `chat_inbound()` handler (`src/generic.rs`): same pattern — after `InboundMessage` constructed, before health check
  - Use `is_empty()` fast-path: skip lock acquisition if no rules
  - TDD: Write integration test with TestServer + guardrail rule that blocks a message → implement → verify 403

  **Must NOT do**:
  - Do not add outbound guardrail check in `send_message()` (v2)
  - Do not add guardrail to WebSocket handler `handle_ws()` (outbound only)
  - Do not modify `InboundMessage` struct
  - Do not modify `drain_buffered_messages()` in health.rs
  - Do not over-abstract: direct evaluation in handlers, no middleware trait

  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: Touches 3 files (server.rs, generic.rs, potentially main.rs), modifies critical request path, requires understanding of AppState lifecycle and handler flow
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 3 (sequential within wave, but wave starts after Wave 2)
  - **Blocks**: Tasks 9, 10, 11
  - **Blocked By**: Tasks 6 (GuardrailEngine), 7 (guardrails_dir)

  **References**:

  **Pattern References**:
  - `src/server.rs:25-36` — `AppState` struct. Add `guardrail_engine` field here.
  - `src/server.rs:500-666` — `adapter_inbound()` handler. Insert guardrail check after `InboundMessage` construction (~line 625), before health state check.
  - `src/generic.rs:64-243` — `chat_inbound()` handler. Insert guardrail check after `InboundMessage` construction (~line 200), before health check.
  - `src/server.rs:42-116` — `create_server()`. Add GuardrailEngine initialization.

  **API/Type References**:
  - `src/guardrail.rs` — `GuardrailEngine::from_rules()`, `evaluate_inbound()`, `GuardrailVerdict`, `is_empty()`. (from Task 6)
  - `src/guardrail.rs` — `load_rules_from_dir()` (from Task 5)
  - `src/error.rs` — `AppError::Forbidden(String)` (from Task 1)

  **Test References**:
  - `tests/integration_test.rs` — TestServer setup pattern, `test_config()` builder, `spawn_mock_backend()`. Use these to create integration tests.

  **Acceptance Criteria**:

  **TDD:**
  - [ ] Integration test: TestServer with block rule → POST inbound with matching text → HTTP 403 + error message
  - [ ] Integration test: TestServer with block rule → POST inbound with non-matching text → HTTP 202
  - [ ] Integration test: TestServer with no rules → POST inbound → HTTP 202 (passthrough)
  - [ ] Integration test: Generic adapter chat_inbound with block rule → HTTP 403
  - [ ] Existing integration tests pass unchanged
  - [ ] `cargo test` passes (unit + integration)

  **QA Scenarios:**

  ```
  Scenario: Inbound message blocked by guardrail
    Tool: Bash (cargo test)
    Steps:
      1. Run `cargo test integration::test_guardrail_blocks_inbound -- --nocapture`
      2. Verify: POST with blocked content → 403 with reject_message in body
    Expected Result: HTTP 403, body contains rule's reject_message
    Failure Indicators: 202 (guardrail not checked), 500 (guardrail error), panic
    Evidence: .sisyphus/evidence/task-8-guardrail-block.txt

  Scenario: Inbound message allowed when no rules match
    Tool: Bash (cargo test)
    Steps:
      1. Run `cargo test integration::test_guardrail_allows_clean_message -- --nocapture`
      2. Verify: POST with clean content → 202
    Expected Result: HTTP 202 Accepted
    Evidence: .sisyphus/evidence/task-8-guardrail-allow.txt

  Scenario: No guardrail rules = passthrough (zero overhead)
    Tool: Bash (cargo test)
    Steps:
      1. Run `cargo test integration::test_no_guardrails_passthrough -- --nocapture`
      2. Verify: TestServer with no guardrails_dir → all messages → 202
    Expected Result: Identical behavior to before guardrails feature
    Evidence: .sisyphus/evidence/task-8-no-guardrails.txt
  ```

  **Commit**: YES (group 5 — with Task 7)
  - Message: `feat(guardrail): wire engine into AppState and intercept inbound handlers`
  - Files: `src/server.rs`, `src/generic.rs`
  - Pre-commit: `cargo test`

- [x] 9. Watcher: monitor guardrails/ directory + rebuild engine

  **What to do**:
  - Modify `watch_config()` in `src/watcher.rs`: accept `guardrails_dir: Option<String>` parameter
  - If `guardrails_dir` is `Some`, add a second `watcher.watch()` call for that directory (`RecursiveMode::NonRecursive`)
  - On guardrails directory events (create/modify/remove .json files): debounce (1000ms, same as config), then reload rules via `load_rules_from_dir()`, compile via `GuardrailEngine::from_rules()`, swap via `*state.guardrail_engine.write().await = new_engine`
  - On parse failure during reload: log error, keep previous valid engine (never replace working rules with broken set)
  - Update `main.rs` to pass `guardrails_dir` to `watch_config()` call
  - TDD: Write test verifying reload behavior → implement → verify

  **Must NOT do**:
  - Do not add recursive watching (rules are flat files in one dir)
  - Do not reload config.json when guardrail files change (separate concerns)
  - Do not add debounce logic from scratch — match existing watcher.rs debounce pattern

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: Modifying existing watcher with new concerns, needs careful debounce logic, touches async runtime
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Tasks 8, 10 — but depends on 8 completing first for AppState access)
  - **Blocks**: None (final feature)
  - **Blocked By**: Tasks 7 (guardrails_dir), 8 (AppState integration)

  **References**:

  **Pattern References**:
  - `src/watcher.rs:18-112` — Existing `watch_config()` function. Shows debounce pattern (1000ms check on line 54), file event handling, `notify::recommended_watcher()` setup. Extend this function, don't create a separate watcher.
  - `src/watcher.rs:54` — Debounce: `if last_reload.elapsed() < Duration::from_millis(1000)` pattern. Use identical timing for guardrails.

  **API/Type References**:
  - `src/guardrail.rs` — `load_rules_from_dir()`, `GuardrailEngine::from_rules()` (from Tasks 5, 6)
  - `src/server.rs` — `state.guardrail_engine` RwLock (from Task 8)

  **Acceptance Criteria**:

  **TDD:**
  - [ ] Test: Modify a rule file → engine is rebuilt with updated rules
  - [ ] Test: Add a new rule file → engine includes new rule
  - [ ] Test: Remove a rule file → engine no longer has that rule
  - [ ] Test: Write malformed JSON → engine keeps previous valid rules (no replacement with empty)
  - [ ] Existing watcher tests pass unchanged
  - [ ] `cargo test` passes

  **QA Scenarios:**

  ```
  Scenario: Guardrail hot-reload on file change
    Tool: Bash (cargo test)
    Steps:
      1. Run `cargo test watcher::tests::test_guardrail_hot_reload -- --nocapture`
      2. Verify: file change detected, engine rebuilt, new rules effective
    Expected Result: Engine contains updated rules after reload
    Evidence: .sisyphus/evidence/task-9-hot-reload.txt

  Scenario: Malformed rule file doesn't break existing rules
    Tool: Bash (cargo test)
    Steps:
      1. Run `cargo test watcher::tests::test_guardrail_reload_malformed_keeps_old -- --nocapture`
      2. Verify: malformed JSON → error logged, previous engine retained
    Expected Result: Previous rules still active, no panic
    Evidence: .sisyphus/evidence/task-9-malformed-no-break.txt
  ```

  **Commit**: YES (group 6)
  - Message: `feat(watcher): monitor guardrails directory for hot-reload`
  - Files: `src/watcher.rs`, `src/main.rs`
  - Pre-commit: `cargo test`

- [x] 10. Integration tests: blocked/allowed messages end-to-end

  **What to do**:
  - Add comprehensive integration tests in `tests/` using existing TestServer pattern
  - Test scenarios: (a) blocked inbound via adapter, (b) blocked inbound via generic, (c) allowed when no match, (d) multiple rules with short-circuit, (e) fail-open on CEL error, (f) disabled rule ignored, (g) empty guardrails dir = passthrough, (h) regex matching via `matches()`
  - Each test creates temp guardrails dir with fixture rule files, configures TestServer, sends requests via reqwest, asserts HTTP status + response body
  - TDD: Tests ARE the deliverable for this task

  **Must NOT do**:
  - Do not add outbound tests (no outbound guardrails in v1)
  - Do not modify any source files
  - Do not test watcher/hot-reload here (Task 9 tests cover that)

  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: Complex test setup with TestServer, temp dirs, multiple scenarios, requires understanding of entire inbound flow
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Task 9 — independent once Task 8 is done)
  - **Blocks**: None
  - **Blocked By**: Task 8 (AppState integration must be complete)

  **References**:

  **Pattern References**:
  - `tests/integration_test.rs` — TestServer setup, `test_config()`, `find_available_port()`, `spawn_mock_backend()`. Follow these patterns exactly for new tests.

  **API/Type References**:
  - `src/config.rs` — `GatewayConfig.guardrails_dir` field (from Task 7)
  - `src/guardrail.rs` — `GuardrailRule` (for creating fixture JSON files)

  **Acceptance Criteria**:

  - [ ] At least 8 integration test scenarios (matching the list above)
  - [ ] All tests use temp directories (no hardcoded paths)
  - [ ] All tests clean up after themselves
  - [ ] `cargo test` passes with all new + existing tests
  - [ ] `cargo clippy --all-targets -- -D warnings` passes

  **QA Scenarios:**

  ```
  Scenario: Full integration test suite passes
    Tool: Bash (cargo test)
    Steps:
      1. Run `cargo test --test integration_test -- guardrail --nocapture`
      2. Verify: all guardrail integration tests pass
    Expected Result: 8+ tests pass, 0 failures
    Failure Indicators: Port conflicts, TestServer setup failure, assertion mismatch
    Evidence: .sisyphus/evidence/task-10-integration-tests.txt
  ```

  **Commit**: YES (group 7)
  - Message: `test(guardrail): integration tests for blocked/allowed message flow`
  - Files: `tests/integration_test.rs` (or `tests/guardrail_test.rs`)
  - Pre-commit: `cargo test`

- [x] 11. Example rule files + config.example.json update + README

  **What to do**:
  - Create `guardrails/` directory at project root with example rule files:
    - `01-block-sensitive-keywords.json` — blocks messages containing password/secret/api_key
    - `02-max-message-length.json` — blocks messages over 10000 chars
    - `03-audit-attachments.json` — logs (action=log) messages with attachments (doesn't block)
  - Update `config.example.json`: add `guardrails_dir` field with comment explaining auto-discovery
  - Update `README.md`: add Guardrails section explaining rule format, CEL expression examples, hot-reload, file ordering
  - Document: `matches()` uses Rust regex syntax (not RE2), `has()` not available, use null checks instead
  - Document: recommended file naming with zero-padded prefix (`01-`, `02-`, ... `10-`)

  **Must NOT do**:
  - Do not write API docs for guardrail internals (not user-facing)
  - Do not document outbound guardrails (not implemented)
  - Do not document LLM guardrails (not implemented)

  **Recommended Agent Profile**:
  - **Category**: `writing`
    - Reason: Documentation and example files, no code changes
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO (final polish, runs after integration proven)
  - **Parallel Group**: Wave 4
  - **Blocks**: None
  - **Blocked By**: Task 8 (need working implementation to verify examples)

  **References**:

  **Pattern References**:
  - `config.example.json` — Existing example config. Add `guardrails_dir` field.
  - `README.md` — Existing sections: Features, Quick Start, Configuration, Adapters, API Endpoints. Add Guardrails section after Adapters.

  **Acceptance Criteria**:

  - [ ] 3 example rule files in `guardrails/` directory
  - [ ] Each example rule is valid JSON that deserializes as `GuardrailRule`
  - [ ] `config.example.json` updated with `guardrails_dir` field
  - [ ] README has Guardrails section with: rule format, CEL examples, limitations, hot-reload docs
  - [ ] README documents `matches()` uses Rust regex syntax
  - [ ] README documents null handling (no `has()`)

  **QA Scenarios:**

  ```
  Scenario: Example rule files are valid
    Tool: Bash
    Steps:
      1. For each file in guardrails/*.json: `python3 -c "import json; json.load(open('guardrails/FILE.json'))"`
      2. Verify: all files parse as valid JSON
    Expected Result: Zero parse errors
    Evidence: .sisyphus/evidence/task-11-example-validation.txt
  ```

  **Commit**: YES (group 8)
  - Message: `docs: add example guardrail rules and update config docs`
  - Files: `guardrails/*.json`, `config.example.json`, `README.md`
  - Pre-commit: `cargo clippy && cargo fmt --check`

---

## Final Verification Wave (MANDATORY — after ALL implementation tasks)

> 4 review agents run in PARALLEL. ALL must APPROVE. Rejection → fix → re-run.

- [x] F1. **Plan Compliance Audit** — `oracle`
  Read the plan end-to-end. For each "Must Have": verify implementation exists (read file, curl endpoint, run command). For each "Must NOT Have": search codebase for forbidden patterns — reject with file:line if found. Check evidence files exist in .sisyphus/evidence/. Compare deliverables against plan.
  Output: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT: APPROVE/REJECT`

- [x] F2. **Code Quality Review** — `unspecified-high`
  Run `cargo clippy --all-targets -- -D warnings` + `cargo fmt --all -- --check` + `cargo test`. Review all changed files for: `as any`, `unwrap()` in non-test code, empty catches, `println!`/`eprintln!` in prod code, commented-out code, unused imports. Check AI slop: excessive comments, over-abstraction, generic variable names.
  Output: `Build [PASS/FAIL] | Lint [PASS/FAIL] | Tests [N pass/N fail] | Files [N clean/N issues] | VERDICT`

- [x] F3. **Real Manual QA** — `unspecified-high`
  Start from clean state. Create test guardrail rules in a temp directory. Start the gateway with those rules. Send inbound messages via curl that should be blocked and allowed. Verify HTTP 403 vs 202 responses. Test hot-reload by adding/modifying/removing rule files while gateway runs. Test invalid CEL expression handling. Save evidence to `.sisyphus/evidence/final-qa/`.
  Output: `Scenarios [N/N pass] | Integration [N/N] | Edge Cases [N tested] | VERDICT`

- [x] F4. **Scope Fidelity Check** — `deep`
  For each task: read "What to do", read actual diff (git log/diff). Verify 1:1 — everything in spec was built (no missing), nothing beyond spec was built (no creep). Check "Must NOT do" compliance. Detect cross-task contamination: Task N touching Task M's files. Flag unaccounted changes.
  Output: `Tasks [N/N compliant] | Contamination [CLEAN/N issues] | Unaccounted [CLEAN/N files] | VERDICT`

---

## Commit Strategy

| Commit | Scope | Message | Pre-commit Gate |
|--------|-------|---------|-----------------|
| 1 | Task 1 | `feat(deps): add cel-interpreter and AppError::Forbidden variant` | `cargo build && cargo test` |
| 2 | Tasks 2-3 | `feat(guardrail): CEL value converter and rule config types with TDD tests` | `cargo test` |
| 3 | Task 4 | `feat(config): XDG-compliant config path resolution` | `cargo test` |
| 4 | Tasks 5-6 | `feat(guardrail): rule file loading and GuardrailEngine with CEL evaluation` | `cargo test` |
| 5 | Tasks 7-8 | `feat(guardrail): wire engine into AppState and intercept inbound handlers` | `cargo test` |
| 6 | Task 9 | `feat(watcher): monitor guardrails directory for hot-reload` | `cargo test` |
| 7 | Task 10 | `test(guardrail): integration tests for blocked/allowed message flow` | `cargo test` |
| 8 | Task 11 | `docs: add example guardrail rules and update config docs` | `cargo clippy && cargo fmt --check` |

---

## Success Criteria

### Verification Commands
```bash
cargo test                                          # Expected: all tests pass
cargo clippy --all-targets -- -D warnings           # Expected: zero warnings
cargo fmt --all -- --check                          # Expected: no formatting changes

# Manual QA: blocked message
curl -X POST http://localhost:8080/api/v1/adapter/inbound \
  -H "Content-Type: application/json" \
  -d '{"instance_id":"...","text":"my password is abc123",...}'
# Expected: 403 Forbidden {"error":"PII detected"}

# Manual QA: allowed message
curl -X POST http://localhost:8080/api/v1/adapter/inbound \
  -H "Content-Type: application/json" \
  -d '{"instance_id":"...","text":"hello world",...}'
# Expected: 202 Accepted
```

### Final Checklist
- [ ] All "Must Have" present
- [ ] All "Must NOT Have" absent
- [ ] All tests pass
- [ ] Config backward compatible (`GATEWAY_CONFIG=./config.json` still works)
- [ ] Guardrail hot-reload works (file change → engine rebuilt)
- [ ] Invalid CEL expression logged and skipped (gateway starts normally)
