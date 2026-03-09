
### [2026-03-09] Task 3: GuardrailRule and Enum Types

#### Implementation Summary
- Added 4 enums to config.rs: GuardrailType, GuardrailAction, GuardrailDirection, GuardrailOnError
- Added GuardrailRule struct with 8 fields
- All enums use `#[derive(Default)]` with `#[default]` attribute on first variant
- All enums use `#[serde(rename_all = "lowercase")]` for JSON compatibility
- GuardrailRule.enabled uses `#[serde(default = "default_true")]` with helper function

#### Key Patterns Applied
- Followed BackendProtocol enum pattern exactly
- Used `#[allow(dead_code)]` on public types (will be used in future tasks)
- TDD approach: wrote 14 tests first, then implemented types
- All 36 config tests pass (12 new guardrail tests + 24 existing)

#### Test Coverage
- Minimal JSON deserialization (required fields only)
- Full JSON deserialization (all fields)
- Default value verification for each field
- Enabled field: default true, explicit false
- Roundtrip serialization/deserialization
- Invalid enum values → serde errors (not panics)

#### Clippy Compliance
- Fixed bool assertion comparisons: `assert_eq!(x, true)` → `assert!(x)`
- Used `#[derive(Default)]` instead of manual impl
- All config.rs code passes clippy -D warnings

#### Evidence
- Test output: .sisyphus/evidence/task-3-rule-defaults-test.txt
- All 36 tests pass in both lib and main test runners

## 2026-03-10: Example guardrail rules + docs

### GuardrailRule serde field names
- `type` field uses `r#type` in Rust but serializes as `"type"` in JSON
- All enum variants use `#[serde(rename_all = "lowercase")]`
- `enabled` defaults to `true` via `default_true()` fn, not `#[serde(default)]` alone
- `reject_message` is `Option<String>` with `#[serde(default)]`

### guardrails_dir auto-discovery
- If `guardrails_dir` is absent from config AND a `guardrails/` dir exists next to config.json, it's picked up automatically via `resolve_guardrails_dir()`
- Relative paths are resolved relative to the config file's parent directory

### Rule file naming
- Files loaded in lexicographic order — zero-padded prefix (`01-`, `02-`) ensures deterministic ordering
- `enabled: false` skips without deleting

### CEL limitations confirmed
- `matches()` uses Rust `regex` crate, NOT RE2 — `(?i)` flag works, lookaheads don't
- `has()` not available in cel-interpreter — use `on_error: "allow"` for fields that may be absent
- `size(message.files) > 0` works for attachment check with `on_error: "allow"` as safety net

### What NOT to document
- Outbound guardrails (not in v1)
- LLM guardrails (not implemented)
- Admin API for guardrails (doesn't exist)
