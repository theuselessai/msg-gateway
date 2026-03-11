# Issues — opencode-backend

## [2026-03-08] Known Issues / Gotchas
- `create_adapter` is called fresh per request at 4 sites — any state in adapter struct is lost between requests
  - Solution: static OnceLock<Arc<RwLock<HashMap<String, String>>>> for session map
- Adapter has no access to gateway URL or send_token by default
  - Solution: Add GatewayContext struct, pass to create_adapter
- Adapter has no access to credential.config (model config)
  - Solution: Add credential_config: Option<&serde_json::Value> param to create_adapter
- health.rs line 349 uses default_target (no credential) — pass None for credential_config there
