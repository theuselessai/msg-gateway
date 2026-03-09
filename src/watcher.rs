//! Config File Watcher
//!
//! Watches the config file for changes and triggers hot reload.
//! Uses debouncing to avoid rapid reloads from multiple file events.

use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::adapter::AdapterInstanceManager;
use crate::config::{self, CredentialConfig};
use crate::guardrail::{GuardrailEngine, load_rules_from_dir};
use crate::manager::CredentialManager;
use crate::server::AppState;

#[derive(Debug)]
enum WatchEvent {
    Config,
    Guardrails,
}

pub async fn watch_config(
    config_path: String,
    state: Arc<AppState>,
    manager: Arc<CredentialManager>,
    adapter_manager: Arc<AdapterInstanceManager>,
    guardrails_dir: Option<String>,
) -> anyhow::Result<()> {
    let (tx, mut rx) = mpsc::channel(10);

    let tx_config = tx.clone();
    let tx_guardrails = tx.clone();
    let mut watcher = RecommendedWatcher::new(
        move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                match event.kind {
                    EventKind::Modify(_) | EventKind::Create(_) => {
                        // notify does not distinguish which registered path triggered
                        // the event, so broadcast both variants; the receiver uses
                        // separate debounce timers and the `watching_guardrails` flag
                        // to ignore irrelevant events.
                        let _ = tx_config.blocking_send(WatchEvent::Config);
                        let _ = tx_guardrails.blocking_send(WatchEvent::Guardrails);
                    }
                    _ => {}
                }
            }
        },
        Config::default(),
    )?;

    let path = Path::new(&config_path);
    let watch_path = path.parent().unwrap_or(Path::new("."));
    watcher.watch(watch_path, RecursiveMode::NonRecursive)?;

    tracing::info!(
        path = %config_path,
        "Config watcher started"
    );

    let watching_guardrails = if let Some(ref dir) = guardrails_dir {
        let guardrails_path = Path::new(dir);
        if guardrails_path.exists() {
            watcher.watch(guardrails_path, RecursiveMode::NonRecursive)?;
            tracing::info!(
                path = %dir,
                "Guardrails watcher started"
            );
            true
        } else {
            tracing::warn!(
                path = %dir,
                "Guardrails directory does not exist, skipping watch"
            );
            false
        }
    } else {
        false
    };

    let debounce_duration = Duration::from_millis(1000);
    let mut last_config_reload = std::time::Instant::now();
    let mut last_guardrail_reload = std::time::Instant::now();

    loop {
        let event = match rx.recv().await {
            Some(e) => e,
            None => break,
        };

        match event {
            WatchEvent::Config => {
                let now = std::time::Instant::now();
                if now.duration_since(last_config_reload) < debounce_duration {
                    continue;
                }

                tokio::time::sleep(Duration::from_millis(100)).await;
                let mut also_reload_guardrails = false;
                while let Ok(drained) = rx.try_recv() {
                    if matches!(drained, WatchEvent::Guardrails) {
                        also_reload_guardrails = true;
                    }
                }

                {
                    let skip_until = state.skip_reload_until.read().await;
                    if let Some(until) = *skip_until
                        && std::time::Instant::now() < until
                    {
                        tracing::debug!("Skipping reload (triggered by Admin API)");
                        continue;
                    }
                }

                tracing::info!("Config file changed, reloading...");

                match config::load_config(&config_path) {
                    Ok(new_config) => {
                        let old_config = state.config.read().await.clone();

                        {
                            let mut config = state.config.write().await;
                            *config = new_config.clone();
                        }

                        sync_adapters(&old_config, &new_config, &manager, &adapter_manager).await;

                        // Sync backend instances with new config
                        sync_backends(&old_config, &new_config, &state).await;

                        tracing::info!("Config reloaded successfully");
                        last_config_reload = std::time::Instant::now();
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to reload config, keeping old config");
                    }
                }

                if also_reload_guardrails
                    && watching_guardrails
                    && let Some(ref dir) = guardrails_dir
                {
                    tracing::info!(path = %dir, "Guardrails directory changed (detected during config drain), reloading rules...");
                    let rules = load_rules_from_dir(Path::new(dir));
                    let new_engine = GuardrailEngine::from_rules(rules);
                    let mut engine = state.guardrail_engine.write().await;
                    *engine = new_engine;
                    tracing::info!(path = %dir, "Guardrail rules reloaded successfully");
                    last_guardrail_reload = std::time::Instant::now();
                }
            }
            WatchEvent::Guardrails => {
                if !watching_guardrails {
                    continue;
                }

                let now = std::time::Instant::now();
                if now.duration_since(last_guardrail_reload) < debounce_duration {
                    continue;
                }

                tokio::time::sleep(Duration::from_millis(100)).await;
                let mut also_reload_config = false;
                while let Ok(drained) = rx.try_recv() {
                    if matches!(drained, WatchEvent::Config) {
                        also_reload_config = true;
                    }
                }

                let dir = guardrails_dir
                    .as_deref()
                    .expect("guardrails_dir is Some when watching_guardrails is true");
                tracing::info!(path = %dir, "Guardrails directory changed, reloading rules...");

                let rules = load_rules_from_dir(Path::new(dir));
                let new_engine = GuardrailEngine::from_rules(rules);

                {
                    let mut engine = state.guardrail_engine.write().await;
                    *engine = new_engine;
                }

                tracing::info!(path = %dir, "Guardrail rules reloaded successfully");
                last_guardrail_reload = std::time::Instant::now();

                if also_reload_config {
                    let skip_until = state.skip_reload_until.read().await;
                    let should_skip = skip_until
                        .map(|until| std::time::Instant::now() < until)
                        .unwrap_or(false);
                    drop(skip_until);

                    if should_skip {
                        tracing::debug!("Skipping config reload (triggered by Admin API)");
                    } else {
                        tracing::info!(
                            "Config file changed (detected during guardrails drain), reloading..."
                        );
                        match config::load_config(&config_path) {
                            Ok(new_config) => {
                                let old_config = state.config.read().await.clone();
                                {
                                    let mut config = state.config.write().await;
                                    *config = new_config.clone();
                                }
                                sync_adapters(&old_config, &new_config, &manager, &adapter_manager)
                                    .await;
                                sync_backends(&old_config, &new_config, &state).await;
                                tracing::info!("Config reloaded successfully");
                                last_config_reload = std::time::Instant::now();
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "Failed to reload config, keeping old config");
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Sync adapter instances with config changes
async fn sync_adapters(
    old_config: &config::Config,
    new_config: &config::Config,
    manager: &Arc<CredentialManager>,
    adapter_manager: &Arc<AdapterInstanceManager>,
) {
    let old_creds = &old_config.credentials;
    let new_creds = &new_config.credentials;

    // Find credentials to stop (removed or deactivated)
    for (id, old_cred) in old_creds {
        match new_creds.get(id) {
            None => {
                // Credential removed
                tracing::info!(credential_id = %id, "Credential removed, stopping adapter");
                adapter_manager.stop(id).await.ok();
                manager.stop_task(id).await;
            }
            Some(new_cred) if !new_cred.active && old_cred.active => {
                // Credential deactivated
                tracing::info!(credential_id = %id, "Credential deactivated, stopping adapter");
                adapter_manager.stop(id).await.ok();
                manager.stop_task(id).await;
            }
            Some(new_cred) if credential_changed(old_cred, new_cred) => {
                // Credential config changed, restart
                tracing::info!(credential_id = %id, "Credential config changed, restarting adapter");
                adapter_manager.stop(id).await.ok();
                manager.stop_task(id).await;

                if new_cred.active {
                    spawn_adapter(id, new_cred, manager, adapter_manager).await;
                }
            }
            _ => {}
        }
    }

    // Find credentials to start (new or activated)
    for (id, new_cred) in new_creds {
        if new_cred.active && !old_creds.contains_key(id) {
            // New credential
            tracing::info!(credential_id = %id, "New credential, starting adapter");
            spawn_adapter(id, new_cred, manager, adapter_manager).await;
        } else if let Some(old_cred) = old_creds.get(id)
            && new_cred.active
            && !old_cred.active
        {
            // Credential activated
            tracing::info!(credential_id = %id, "Credential activated, starting adapter");
            spawn_adapter(id, new_cred, manager, adapter_manager).await;
        }
    }
}

/// Spawn an adapter instance for a credential
async fn spawn_adapter(
    credential_id: &str,
    cred_config: &CredentialConfig,
    manager: &Arc<CredentialManager>,
    adapter_manager: &Arc<AdapterInstanceManager>,
) {
    match adapter_manager
        .spawn(
            credential_id,
            &cred_config.adapter,
            &cred_config.token,
            cred_config.config.as_ref(),
        )
        .await
    {
        Ok((instance_id, port)) => {
            tracing::info!(
                credential_id = %credential_id,
                adapter = %cred_config.adapter,
                instance_id = %instance_id,
                port = %port,
                "Adapter instance started"
            );

            // For external adapters, wait for them to become healthy
            if cred_config.adapter != "generic" && port > 0 {
                let ready = crate::adapter::wait_for_adapter_ready(
                    adapter_manager,
                    credential_id,
                    std::time::Duration::from_secs(30),
                    std::time::Duration::from_millis(500),
                )
                .await;

                if ready {
                    tracing::info!(
                        credential_id = %credential_id,
                        "Adapter is ready"
                    );
                } else {
                    tracing::warn!(
                        credential_id = %credential_id,
                        "Adapter did not become ready within timeout"
                    );
                }
            }

            // Register in credential manager
            manager
                .spawn_task(credential_id.to_string(), cred_config.clone())
                .await;
        }
        Err(e) => {
            tracing::error!(
                credential_id = %credential_id,
                adapter = %cred_config.adapter,
                error = %e,
                "Failed to start adapter instance"
            );
        }
    }
}

async fn sync_backends(
    old_config: &config::Config,
    new_config: &config::Config,
    state: &Arc<AppState>,
) {
    use crate::config::BackendProtocol;

    let old_backends = &old_config.backends;
    let new_backends = &new_config.backends;

    // Stop removed or changed backends
    for (name, old_cfg) in old_backends {
        match new_backends.get(name) {
            None => {
                tracing::info!(backend = %name, "Backend removed, stopping");
                state.backend_manager.stop(name).await;
            }
            Some(new_cfg) if old_cfg != new_cfg => {
                if old_cfg.active && old_cfg.protocol == BackendProtocol::External {
                    tracing::info!(backend = %name, "Backend config changed, restarting");
                    state.backend_manager.stop(name).await;
                }
                if new_cfg.active && new_cfg.protocol == BackendProtocol::External {
                    spawn_backend(name, new_cfg, state).await;
                }
            }
            _ => {}
        }
    }

    // Spawn newly added backends
    for (name, new_cfg) in new_backends {
        if !old_backends.contains_key(name)
            && new_cfg.active
            && new_cfg.protocol == BackendProtocol::External
        {
            tracing::info!(backend = %name, "New backend, spawning");
            spawn_backend(name, new_cfg, state).await;
        }
    }
}

async fn spawn_backend(
    name: &str,
    backend_cfg: &crate::config::BackendConfig,
    state: &Arc<AppState>,
) {
    match state.backend_manager.spawn(name, backend_cfg).await {
        Ok((port, token)) => {
            tracing::info!(backend = %name, port = %port, "External backend spawned");

            let ready = crate::backend::wait_for_backend_ready(
                port,
                std::time::Duration::from_secs(30),
                std::time::Duration::from_millis(500),
            )
            .await;

            if ready {
                tracing::info!(backend = %name, "Backend is ready");
            } else {
                tracing::warn!(backend = %name, "Backend did not become ready within timeout");
            }

            let mut cfg = state.config.write().await;
            if let Some(b) = cfg.backends.get_mut(name) {
                b.port = Some(port);
                b.token = token;
            }
        }
        Err(e) => {
            tracing::error!(backend = %name, error = %e, "Failed to spawn external backend");
        }
    }
}

fn credential_changed(old: &CredentialConfig, new: &CredentialConfig) -> bool {
    old.adapter != new.adapter || old.token != new.token || old.config != new.config
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AuthConfig, Config, CredentialConfig, GatewayConfig};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_credential(adapter: &str, token: &str, active: bool) -> CredentialConfig {
        CredentialConfig {
            adapter: adapter.to_string(),
            token: token.to_string(),
            active,
            emergency: false,
            config: None,
            backend: None,
            route: serde_json::json!({"test": true}),
        }
    }

    fn make_config(credentials: Vec<(&str, CredentialConfig)>) -> Config {
        Config {
            gateway: GatewayConfig {
                listen: "127.0.0.1:8080".to_string(),
                admin_token: "test-admin-token".to_string(),
                default_backend: None,
                adapters_dir: "./adapters".to_string(),
                adapter_port_range: (9000, 9100),
                backends_dir: "./backends".to_string(),
                backend_port_range: (9200, 9300),
                file_cache: None,
                guardrails_dir: None,
            },
            auth: AuthConfig {
                send_token: "test-send-token".to_string(),
            },
            health_checks: HashMap::new(),
            credentials: credentials
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            backends: HashMap::new(),
        }
    }

    fn make_adapter_manager() -> Arc<AdapterInstanceManager> {
        Arc::new(
            AdapterInstanceManager::new("./adapters".to_string(), (19000, 19100), "127.0.0.1:8080")
                .unwrap(),
        )
    }

    #[test]
    fn test_credential_changed_same() {
        let cred1 = make_credential("telegram", "token123", true);
        let cred2 = make_credential("telegram", "token123", true);
        assert!(!credential_changed(&cred1, &cred2));
    }

    #[test]
    fn test_credential_changed_adapter() {
        let cred1 = make_credential("telegram", "token123", true);
        let cred2 = make_credential("discord", "token123", true);
        assert!(credential_changed(&cred1, &cred2));
    }

    #[test]
    fn test_credential_changed_token() {
        let cred1 = make_credential("telegram", "token123", true);
        let cred2 = make_credential("telegram", "token456", true);
        assert!(credential_changed(&cred1, &cred2));
    }

    #[test]
    fn test_credential_changed_active_not_considered() {
        // Active status change should NOT trigger restart (handled separately)
        let cred1 = make_credential("telegram", "token123", true);
        let cred2 = make_credential("telegram", "token123", false);
        assert!(!credential_changed(&cred1, &cred2));
    }

    #[test]
    fn test_credential_changed_config() {
        let mut cred1 = make_credential("telegram", "token123", true);
        let mut cred2 = make_credential("telegram", "token123", true);

        cred1.config = Some(serde_json::json!({"key": "value1"}));
        cred2.config = Some(serde_json::json!({"key": "value2"}));

        assert!(credential_changed(&cred1, &cred2));
    }

    #[test]
    fn test_credential_changed_config_none_to_some() {
        let mut cred1 = make_credential("telegram", "token123", true);
        let mut cred2 = make_credential("telegram", "token123", true);

        cred1.config = None;
        cred2.config = Some(serde_json::json!({"key": "value"}));

        assert!(credential_changed(&cred1, &cred2));
    }

    // Helper to check if credential is registered in the manager
    async fn is_registered(manager: &CredentialManager, credential_id: &str) -> bool {
        manager.registry.is_running(credential_id).await
    }

    #[tokio::test]
    async fn test_sync_adapters_new_credential() {
        let manager = Arc::new(CredentialManager::new());
        let adapter_manager = make_adapter_manager();

        let old_config = make_config(vec![]);
        let new_config = make_config(vec![("cred1", make_credential("generic", "token1", true))]);

        sync_adapters(&old_config, &new_config, &manager, &adapter_manager).await;

        // Generic adapter should be registered in the credential manager
        assert!(is_registered(&manager, "cred1").await);
    }

    #[tokio::test]
    async fn test_sync_adapters_removed_credential() {
        let manager = Arc::new(CredentialManager::new());
        let adapter_manager = make_adapter_manager();

        // First add a credential
        let old_config = make_config(vec![("cred1", make_credential("generic", "token1", true))]);
        let empty_config = make_config(vec![]);

        // Manually spawn to simulate existing state
        manager
            .spawn_task(
                "cred1".to_string(),
                make_credential("generic", "token1", true),
            )
            .await;

        // Now sync with empty config (credential removed)
        sync_adapters(&old_config, &empty_config, &manager, &adapter_manager).await;

        // Should be removed from credential manager
        assert!(!is_registered(&manager, "cred1").await);
    }

    #[tokio::test]
    async fn test_sync_adapters_deactivated_credential() {
        let manager = Arc::new(CredentialManager::new());
        let adapter_manager = make_adapter_manager();

        let old_config = make_config(vec![("cred1", make_credential("generic", "token1", true))]);
        let new_config = make_config(vec![("cred1", make_credential("generic", "token1", false))]);

        // Manually spawn to simulate existing state
        manager
            .spawn_task(
                "cred1".to_string(),
                make_credential("generic", "token1", true),
            )
            .await;

        sync_adapters(&old_config, &new_config, &manager, &adapter_manager).await;

        // Should be stopped (removed from credential manager)
        assert!(!is_registered(&manager, "cred1").await);
    }

    #[tokio::test]
    async fn test_sync_adapters_activated_credential() {
        let manager = Arc::new(CredentialManager::new());
        let adapter_manager = make_adapter_manager();

        let old_config = make_config(vec![("cred1", make_credential("generic", "token1", false))]);
        let new_config = make_config(vec![("cred1", make_credential("generic", "token1", true))]);

        sync_adapters(&old_config, &new_config, &manager, &adapter_manager).await;

        // Should be started (registered in credential manager)
        assert!(is_registered(&manager, "cred1").await);
    }

    #[tokio::test]
    async fn test_sync_adapters_config_changed() {
        let manager = Arc::new(CredentialManager::new());
        let adapter_manager = make_adapter_manager();

        let old_config = make_config(vec![("cred1", make_credential("generic", "token1", true))]);

        let new_cred = make_credential("generic", "token2", true); // token changed
        let new_config = make_config(vec![("cred1", new_cred)]);

        // Manually spawn to simulate existing state
        manager
            .spawn_task(
                "cred1".to_string(),
                make_credential("generic", "token1", true),
            )
            .await;

        sync_adapters(&old_config, &new_config, &manager, &adapter_manager).await;

        // Should be restarted (still registered)
        assert!(is_registered(&manager, "cred1").await);
    }

    #[tokio::test]
    async fn test_spawn_adapter_generic() {
        let manager = Arc::new(CredentialManager::new());
        let adapter_manager = make_adapter_manager();

        let cred = make_credential("generic", "token123", true);
        spawn_adapter("test_cred", &cred, &manager, &adapter_manager).await;

        // Generic adapter should be registered in credential manager
        assert!(is_registered(&manager, "test_cred").await);
    }

    #[tokio::test]
    async fn test_spawn_adapter_external_not_found() {
        let manager = Arc::new(CredentialManager::new());
        let adapter_manager = make_adapter_manager();

        // Use a non-existent adapter name (doesn't exist in adapters dir)
        let cred = make_credential("nonexistent_adapter", "token123", true);
        spawn_adapter("test_cred", &cred, &manager, &adapter_manager).await;

        // External adapter spawn should fail (adapter not found), nothing registered
        assert!(!is_registered(&manager, "test_cred").await);
    }

    #[tokio::test]
    async fn test_sync_adapters_multiple_changes() {
        let manager = Arc::new(CredentialManager::new());
        let adapter_manager = make_adapter_manager();

        let old_config = make_config(vec![
            ("keep", make_credential("generic", "token1", true)),
            ("remove", make_credential("generic", "token2", true)),
            ("deactivate", make_credential("generic", "token3", true)),
        ]);

        let new_config = make_config(vec![
            ("keep", make_credential("generic", "token1", true)),
            ("deactivate", make_credential("generic", "token3", false)),
            ("new", make_credential("generic", "token4", true)),
        ]);

        // Setup initial state - only register in credential manager (generic adapters)
        for (id, cred) in &old_config.credentials {
            if cred.active {
                manager.spawn_task(id.clone(), cred.clone()).await;
            }
        }

        sync_adapters(&old_config, &new_config, &manager, &adapter_manager).await;

        // Check expected state:
        // - keep: should still be running (unchanged)
        // - remove: should be stopped (removed from config)
        // - deactivate: should be stopped (deactivated)
        // - new: should be running (newly added)
        assert!(is_registered(&manager, "keep").await);
        assert!(!is_registered(&manager, "remove").await);
        assert!(!is_registered(&manager, "deactivate").await);
        assert!(is_registered(&manager, "new").await);
    }

    #[tokio::test]
    async fn test_sync_adapters_unchanged_credential() {
        let manager = Arc::new(CredentialManager::new());
        let adapter_manager = make_adapter_manager();

        let config = make_config(vec![("cred1", make_credential("generic", "token1", true))]);

        // Setup initial state
        manager
            .spawn_task(
                "cred1".to_string(),
                make_credential("generic", "token1", true),
            )
            .await;

        // Sync with same config (no changes)
        sync_adapters(&config, &config, &manager, &adapter_manager).await;

        // Should still have the same instance
        assert!(is_registered(&manager, "cred1").await);
    }

    #[test]
    fn test_guardrail_reload_valid_rules_replaces_engine() {
        use crate::guardrail::{GuardrailEngine, load_rules_from_dir};
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let rule_json = r#"{"name":"block_all","expression":"true","enabled":true}"#;
        fs::write(dir.path().join("01_rule.json"), rule_json).unwrap();

        let rules = load_rules_from_dir(dir.path());
        assert_eq!(rules.len(), 1);

        let engine = GuardrailEngine::from_rules(rules);
        assert!(!engine.is_empty());
    }

    #[test]
    fn test_guardrail_reload_malformed_json_keeps_valid_rules() {
        use crate::guardrail::{GuardrailEngine, load_rules_from_dir};
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let valid_rule = r#"{"name":"allow_rule","expression":"false","enabled":true}"#;
        fs::write(dir.path().join("01_valid.json"), valid_rule).unwrap();
        fs::write(dir.path().join("02_bad.json"), "not valid json {{{").unwrap();

        let rules = load_rules_from_dir(dir.path());
        assert_eq!(
            rules.len(),
            1,
            "malformed file should be skipped, valid rule kept"
        );

        let engine = GuardrailEngine::from_rules(rules);
        assert!(!engine.is_empty());
    }
}
