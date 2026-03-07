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
