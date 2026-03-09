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
use crate::config::{self, BackendProtocol, CredentialConfig};
use crate::manager::CredentialManager;
use crate::server::AppState;

/// Start watching the config file for changes
pub async fn watch_config(
    config_path: String,
    state: Arc<AppState>,
    manager: Arc<CredentialManager>,
    adapter_manager: Arc<AdapterInstanceManager>,
) -> anyhow::Result<()> {
    let (tx, mut rx) = mpsc::channel(10);

    // Create watcher
    let mut watcher = RecommendedWatcher::new(
        move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                // Only care about modify/create events
                match event.kind {
                    EventKind::Modify(_) | EventKind::Create(_) => {
                        let _ = tx.blocking_send(());
                    }
                    _ => {}
                }
            }
        },
        Config::default(),
    )?;

    // Watch the config file's parent directory
    let path = Path::new(&config_path);
    let watch_path = path.parent().unwrap_or(Path::new("."));

    watcher.watch(watch_path, RecursiveMode::NonRecursive)?;

    tracing::info!(
        path = %config_path,
        "Config watcher started"
    );

    // Debounce timer - long enough to avoid race with Admin API
    let debounce_duration = Duration::from_millis(1000);
    let mut last_reload = std::time::Instant::now();

    loop {
        // Wait for file change event
        if rx.recv().await.is_none() {
            break;
        }

        // Debounce: skip if we just reloaded
        let now = std::time::Instant::now();
        if now.duration_since(last_reload) < debounce_duration {
            continue;
        }

        // Additional delay to let Admin API operations complete
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Drain any pending events (more debouncing)
        while rx.try_recv().is_ok() {}

        // Check if this change was triggered by Admin API
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

        // Load new config
        match config::load_config(&config_path) {
            Ok(new_config) => {
                let old_config = state.config.read().await.clone();

                // Update config in state
                {
                    let mut config = state.config.write().await;
                    *config = new_config.clone();
                }

                // Sync adapter instances with new config
                sync_adapters(&old_config, &new_config, &manager, &adapter_manager).await;
                sync_backend(&old_config, &new_config, &state).await;

                tracing::info!("Config reloaded successfully");
                last_reload = std::time::Instant::now();
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to reload config, keeping old config");
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

/// Check if credential config has changed in a way that requires instance restart
fn credential_changed(old: &CredentialConfig, new: &CredentialConfig) -> bool {
    old.adapter != new.adapter || old.token != new.token || old.config != new.config
}

async fn sync_backend(
    old_config: &config::Config,
    new_config: &config::Config,
    state: &Arc<AppState>,
) {
    let old_target = &old_config.gateway.default_target;
    let new_target = &new_config.gateway.default_target;

    if old_target == new_target {
        return;
    }

    tracing::info!("Backend target config changed, resyncing...");

    let backend_manager = &state.backend_manager;

    if old_target.protocol == BackendProtocol::External {
        backend_manager.stop("__default_backend__").await;
    }

    if new_target.protocol == BackendProtocol::External {
        let adapter_dir = new_target.adapter_dir.as_deref();
        let backend_config = new_config
            .credentials
            .values()
            .find(|c| c.active)
            .and_then(|c| c.config.as_ref());

        match backend_manager
            .spawn(
                "__default_backend__",
                adapter_dir,
                "opencode",
                &new_target.token,
                backend_config,
            )
            .await
        {
            Ok((port, token)) => {
                tracing::info!(port = %port, "External backend adapter respawned");

                let ready = crate::backend::wait_for_backend_ready(
                    port,
                    std::time::Duration::from_secs(30),
                    std::time::Duration::from_millis(500),
                )
                .await;

                if ready {
                    tracing::info!("External backend adapter is ready");
                } else {
                    tracing::warn!("External backend adapter did not become ready within timeout");
                }

                {
                    let mut cfg = state.config.write().await;
                    cfg.gateway.default_target.port = Some(port);
                    cfg.gateway.default_target.token = token;
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to respawn external backend adapter");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        AuthConfig, BackendProtocol, Config, CredentialConfig, GatewayConfig, TargetConfig,
    };
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_credential(adapter: &str, token: &str, active: bool) -> CredentialConfig {
        CredentialConfig {
            adapter: adapter.to_string(),
            token: token.to_string(),
            active,
            emergency: false,
            config: None,
            target: None,
            route: serde_json::json!({"test": true}),
        }
    }

    fn make_config(credentials: Vec<(&str, CredentialConfig)>) -> Config {
        Config {
            gateway: GatewayConfig {
                listen: "127.0.0.1:8080".to_string(),
                admin_token: "test-admin-token".to_string(),
                default_target: TargetConfig {
                    protocol: BackendProtocol::Pipelit,
                    inbound_url: Some("http://localhost:9000/inbound".to_string()),
                    base_url: None,
                    token: "test-backend-token".to_string(),
                    poll_interval_ms: None,
                    adapter_dir: None,
                    port: None,
                },
                adapters_dir: "./adapters".to_string(),
                adapter_port_range: (9000, 9100),
                backends_dir: "./backends".to_string(),
                backend_port_range: (9200, 9300),
                file_cache: None,
            },
            auth: AuthConfig {
                send_token: "test-send-token".to_string(),
            },
            health_checks: HashMap::new(),
            credentials: credentials
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
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
}
