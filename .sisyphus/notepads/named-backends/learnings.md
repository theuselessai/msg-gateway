# Learnings — named-backends

## Current state (before this plan)

- `ExternalBackendManager.spawn()` already uses `HashMap<String, ExternalBackendProcess>` — the key is currently `credential_id` / sentinel `"__default_backend__"`. Changing it to backend name is minimal.
- `TargetConfig` has `port: Option<u16>` and `token: String` — these are written back at runtime after spawn. Same pattern will apply to `BackendConfig`.
- `BackendConfig.token` for External backends = runtime-generated UUID. Config file can leave it empty `""` or omit (serde default).
- `BackendConfig.config` (new field) replaces the hack in `main.rs` that grabbed `credential.config` from the first active credential to pass as `BACKEND_CONFIG`.
- The watcher `sync_backends()` should follow the exact same diff pattern as `sync_adapters()` — it already handles all cases (added, removed, changed, deactivated).
- `resolve_backend_name` returns `Option<String>`, caller errors if `None` (neither credential.backend nor default_backend set).

## T2 learnings (config schema)

- `TargetConfig` → `BackendConfig` rename required a `pub type TargetConfig = BackendConfig;` alias because 5 other files reference the type name.
- Could NOT fully remove `GatewayConfig.default_target` or `CredentialConfig.target` — runtime code in server.rs, generic.rs, health.rs, backend.rs access these fields directly. Kept both old + new fields with TODO comments for T3–T6 cleanup.
- `default_target` changed from required to `#[serde(default = "default_backend_config")]` so configs without it still parse. The default is a dummy Pipelit config with empty token.
- `cargo check` only validates non-test code; `cargo test --lib` also compiles `#[cfg(test)]` blocks in ALL modules. Test constructors in admin.rs, backend.rs, manager.rs, watcher.rs all needed the new fields added.
- admin.rs is NOT in the restricted file list but still needed `backend: None` added to struct construction in both runtime and test code.
- `BackendConfig` derives `PartialEq` (needed by T5 watcher for change detection). `serde_json::Value` already implements `PartialEq` so no issues.
- `BackendConfig.active` defaults to `true` via `#[serde(default = "default_true")]` — omitting from JSON means the backend is active.

## T3–T6 learnings (lifecycle, routing, hot-reload)

- Removed all deprecated shims in one pass: `GatewayConfig.default_target`, `CredentialConfig.target`, `pub type TargetConfig = BackendConfig;`, `default_backend_config()`. This rippled to 9 files total (config, backend, server, main, watcher, generic, health, admin, manager).
- `integration_test.rs` was ALREADY broken from T2 (missing `backend` field on CredentialConfig, missing `backends` on Config, `TargetConfig` still referenced). Confirmed not to touch — T7 handles it.
- `admin.rs` `target: Option<TargetConfig>` on Create/UpdateCredentialRequest changed to `backend: Option<String>`. All admin test assertions updated accordingly.
- `health.rs` `drain_buffered_messages` previously fell back to `default_target` when credential was removed. Now drops the message with a warning since there's no way to route without a credential → backend mapping. This is correct behavior.
- `ExternalBackendManager.spawn()` signature simplified from 5 params to 2: `(backend_name, backend_cfg)`. The adapter_dir comes from `backend_cfg.adapter_dir`, config from `backend_cfg.config`. HashMap key changed from credential_id to backend_name.
- `stop_all()` uses `drain()` on the HashMap for clean iteration + removal.
- `wait_for_backend_ready()` is a standalone function (not a method) — cleaner API, used from both `main.rs` and `watcher.rs`.
- `sync_backends()` in watcher.rs mirrors `sync_adapters()` pattern: diff old vs new, stop removed/changed, spawn added/changed. Writes back port/token to state config after spawn.
- `cargo test --lib` runs 237 tests. `cargo clippy -- -D warnings` and `cargo fmt --check` both pass clean.

## Live config.json migration (T8)

Before (old schema):
```json
{
  "gateway": {
    "default_target": {
      "protocol": "external",
      "adapter_dir": "./backends/opencode",
      "port": 9200,
      "token": "backend-token-12345"
    }
  },
  "credentials": {
    "telegram": {
      "config": { "base_url": "http://127.0.0.1:4096", "model": {...} }
    }
  }
}
```

After (new schema):
```json
{
  "gateway": {
    "default_backend": "opencode"
  },
  "backends": {
    "opencode": {
      "protocol": "external",
      "adapter_dir": "./backends/opencode",
      "active": true,
      "token": "",
      "config": {
        "poll_timeout": 30,
        "base_url": "http://127.0.0.1:4096",
        "token": ":",
        "model": { "providerID": "anthropic", "modelID": "claude-sonnet-4-5" }
      }
    }
  },
  "credentials": {
    "telegram": {
      "backend": "opencode",
      "config": { "poll_timeout": 30 }
    }
  }
}
```

Key changes:
1. Move gateway.default_target → backends.<name>
2. Move credential.config OpenCode settings → backends.<name>.config
3. Add credential.backend = "<name>" (or rely on gateway.default_backend)
4. gateway.default_target is gone; use default_backend instead
