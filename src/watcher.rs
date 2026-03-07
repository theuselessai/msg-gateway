//! Config File Watcher
//!
//! Watches the config file for changes and triggers hot reload.
//! Uses debouncing to avoid rapid reloads from multiple file events.

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher, EventKind};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::config;
use crate::manager::CredentialManager;
use crate::server::AppState;

/// Start watching the config file for changes
pub async fn watch_config(
    config_path: String,
    state: Arc<AppState>,
    manager: Arc<CredentialManager>,
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
            if let Some(until) = *skip_until {
                if std::time::Instant::now() < until {
                    tracing::debug!("Skipping reload (triggered by Admin API)");
                    continue;
                }
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

                // Sync credential tasks
                manager.sync_with_config(&old_config, &new_config).await;

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
