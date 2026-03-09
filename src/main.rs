mod adapter;
mod admin;
mod backend;
mod config;
mod error;
mod files;
mod generic;
mod health;
mod manager;
mod message;
mod server;
mod watcher;

use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::adapter::AdapterInstanceManager;
use crate::backend::ExternalBackendManager;
use crate::config::BackendProtocol;
use crate::manager::CredentialManager;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "msg_gateway=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting msg-gateway");

    // Load config
    let config_path = std::env::var("GATEWAY_CONFIG").unwrap_or_else(|_| "config.json".to_string());

    let config = config::load_config(&config_path)?;
    tracing::info!(listen = %config.gateway.listen, "Configuration loaded");

    // Create adapter instance manager
    let adapter_manager = Arc::new(AdapterInstanceManager::new(
        config.gateway.adapters_dir.clone(),
        config.gateway.adapter_port_range,
        &config.gateway.listen,
    )?);
    tracing::info!(
        adapters_dir = %config.gateway.adapters_dir,
        adapters_found = adapter_manager.adapters.len(),
        "Adapter manager initialized"
    );

    // Create backend manager for external backend adapters
    let backend_manager = Arc::new(ExternalBackendManager::new(
        config.gateway.backends_dir.clone(),
        config.gateway.backend_port_range,
        &config.gateway.listen,
        config.auth.send_token.clone(),
    ));

    // Create credential manager
    let manager = Arc::new(CredentialManager::new());

    // Start HTTP server (before spawning adapters/backends, so they can connect)
    let manager_clone = manager.clone();
    let adapter_manager_clone = adapter_manager.clone();
    let backend_manager_clone = backend_manager.clone();
    let (state, server_future) = server::create_server(
        config.clone(),
        manager_clone,
        adapter_manager_clone,
        backend_manager_clone,
    )
    .await?;

    // Start adapter instances for all active credentials
    for (credential_id, cred_config) in &config.credentials {
        if cred_config.active {
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
                        let ready = adapter::wait_for_adapter_ready(
                            &adapter_manager,
                            credential_id,
                            Duration::from_secs(30),
                            Duration::from_millis(500),
                        )
                        .await;

                        if ready {
                            tracing::info!(
                                credential_id = %credential_id,
                                adapter = %cred_config.adapter,
                                "Adapter is ready"
                            );
                        } else {
                            tracing::warn!(
                                credential_id = %credential_id,
                                adapter = %cred_config.adapter,
                                "Adapter did not become ready within timeout"
                            );
                        }
                    }

                    // Register in credential manager
                    manager
                        .spawn_task(credential_id.clone(), cred_config.clone())
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
    }

    // Spawn all active External backends
    for (name, backend_cfg) in &config.backends {
        if backend_cfg.active && backend_cfg.protocol == BackendProtocol::External {
            match backend_manager.spawn(name, backend_cfg).await {
                Ok((port, token)) => {
                    tracing::info!(
                        backend = %name,
                        port = %port,
                        "External backend spawned"
                    );

                    let ready = backend::wait_for_backend_ready(
                        port,
                        Duration::from_secs(30),
                        Duration::from_millis(500),
                    )
                    .await;

                    if ready {
                        tracing::info!(backend = %name, "Backend is ready");
                    } else {
                        tracing::warn!(backend = %name, "Backend did not become ready within timeout");
                    }

                    // Write back runtime port + token
                    let mut cfg = state.config.write().await;
                    if let Some(b) = cfg.backends.get_mut(name) {
                        b.port = Some(port);
                        b.token = token;
                    }
                }
                Err(e) => {
                    tracing::error!(
                        backend = %name,
                        error = %e,
                        "Failed to spawn external backend"
                    );
                }
            }
        }
    }

    // Start adapter health monitor in background (check every 30s, max 3 failures)
    {
        let adapter_manager_for_health = adapter_manager.clone();
        tokio::spawn(async move {
            adapter::start_adapter_health_monitor(
                adapter_manager_for_health,
                30, // interval_secs
                3,  // max_failures
            )
            .await;
        });
    }

    // Start config watcher in background
    let watcher_state = state.clone();
    let watcher_manager = manager.clone();
    let watcher_adapter_manager = adapter_manager.clone();
    let watcher_path = config_path.clone();
    tokio::spawn(async move {
        if let Err(e) = watcher::watch_config(
            watcher_path,
            watcher_state,
            watcher_manager,
            watcher_adapter_manager,
        )
        .await
        {
            tracing::error!(error = %e, "Config watcher failed");
        }
    });

    // Start health checks for configured targets
    {
        let config = state.config.read().await;
        for (name, health_config) in &config.health_checks {
            let state_clone = state.clone();
            let name_clone = name.clone();
            let config_clone = health_config.clone();
            tokio::spawn(async move {
                health::start_health_check(state_clone, name_clone, config_clone).await;
            });
        }
    }

    // Run server (blocks until shutdown)
    server_future.await?;

    // Graceful shutdown
    tracing::info!("Shutting down...");
    backend_manager.stop_all().await;
    adapter_manager.stop_all().await;
    manager.shutdown().await;

    Ok(())
}
