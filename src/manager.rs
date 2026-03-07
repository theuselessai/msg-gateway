//! Credential Manager
//!
//! Manages the lifecycle of adapter instances:
//! - Spawns adapter processes for active credentials on startup
//! - Monitors config changes and adjusts running instances
//! - Gracefully shuts down instances when credentials are deactivated/removed

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::config::{Config, CredentialConfig};

/// Status of an adapter instance
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceStatus {
    Starting,
    Running,
    Stopping,
    Stopped,
    Error(String),
}

/// Info about a running adapter instance
pub struct InstanceInfo {
    pub instance_id: String,
    pub credential_id: String,
    pub adapter: String,
    pub port: u16,
    pub status: InstanceStatus,
    // TODO: Add process handle, etc.
}

/// Registry of running adapter instances
pub struct TaskRegistry {
    instances: RwLock<HashMap<String, InstanceInfo>>,  // credential_id -> instance
}

impl TaskRegistry {
    pub fn new() -> Self {
        Self {
            instances: RwLock::new(HashMap::new()),
        }
    }

    /// Get status of all instances
    pub async fn get_all_status(&self) -> HashMap<String, (String, InstanceStatus)> {
        let instances = self.instances.read().await;
        instances
            .iter()
            .map(|(id, info)| (id.clone(), (info.adapter.clone(), info.status.clone())))
            .collect()
    }

    /// Get status of a specific instance
    pub async fn get_status(&self, credential_id: &str) -> Option<InstanceStatus> {
        let instances = self.instances.read().await;
        instances.get(credential_id).map(|info| info.status.clone())
    }

    /// Check if an instance is running
    pub async fn is_running(&self, credential_id: &str) -> bool {
        let instances = self.instances.read().await;
        instances
            .get(credential_id)
            .map(|info| info.status == InstanceStatus::Running)
            .unwrap_or(false)
    }

    /// Get instance by credential_id
    pub async fn get_instance(&self, credential_id: &str) -> Option<(String, u16)> {
        let instances = self.instances.read().await;
        instances.get(credential_id).map(|info| (info.instance_id.clone(), info.port))
    }

    /// Register a new instance
    pub async fn register(
        &self,
        credential_id: String,
        instance_id: String,
        adapter: String,
        port: u16,
    ) {
        let mut instances = self.instances.write().await;
        instances.insert(
            credential_id.clone(),
            InstanceInfo {
                instance_id,
                credential_id,
                adapter,
                port,
                status: InstanceStatus::Running,
            },
        );
    }

    /// Update instance status
    pub async fn update_status(&self, credential_id: &str, status: InstanceStatus) {
        let mut instances = self.instances.write().await;
        if let Some(info) = instances.get_mut(credential_id) {
            info.status = status;
        }
    }

    /// Remove an instance from registry
    pub async fn remove(&self, credential_id: &str) -> Option<InstanceInfo> {
        let mut instances = self.instances.write().await;
        instances.remove(credential_id)
    }
}

/// Credential Manager handles spawning/stopping adapter instances based on config
pub struct CredentialManager {
    pub registry: Arc<TaskRegistry>,
}

impl CredentialManager {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(TaskRegistry::new()),
        }
    }

    /// Spawn instances for all active credentials in config
    pub async fn start_all(&self, config: &Config) {
        for (credential_id, cred_config) in &config.credentials {
            if cred_config.active {
                self.spawn_task(credential_id.clone(), cred_config.clone())
                    .await;
            } else {
                tracing::debug!(
                    credential_id = %credential_id,
                    "Skipping inactive credential"
                );
            }
        }
    }

    /// Spawn an adapter instance for a credential
    pub async fn spawn_task(&self, credential_id: String, config: CredentialConfig) {
        // Check if already running
        if self.registry.is_running(&credential_id).await {
            tracing::warn!(
                credential_id = %credential_id,
                "Instance already running, skipping spawn"
            );
            return;
        }

        let adapter = &config.adapter;

        if adapter == "generic" {
            // Generic adapter is built-in, no external process needed
            let instance_id = format!("generic_{}", uuid::Uuid::new_v4());
            self.registry
                .register(credential_id.clone(), instance_id, "generic".to_string(), 0)
                .await;

            tracing::info!(
                credential_id = %credential_id,
                adapter = "generic",
                "Generic adapter ready (no background process needed)"
            );
        } else {
            // TODO: Spawn external adapter process
            // For now, just register a placeholder
            let instance_id = format!("{}_{}", adapter, uuid::Uuid::new_v4());
            let port = 0; // TODO: Allocate port from range

            self.registry
                .register(credential_id.clone(), instance_id, adapter.clone(), port)
                .await;

            tracing::info!(
                credential_id = %credential_id,
                adapter = %adapter,
                "External adapter registered (process spawn not yet implemented)"
            );
        }
    }

    /// Stop a specific credential's adapter instance
    pub async fn stop_task(&self, credential_id: &str) {
        if let Some(info) = self.registry.remove(credential_id).await {
            if info.adapter != "generic" {
                // TODO: Send SIGTERM to process
                tracing::info!(
                    credential_id = %credential_id,
                    adapter = %info.adapter,
                    "Adapter instance stopped (process termination not yet implemented)"
                );
            } else {
                tracing::info!(
                    credential_id = %credential_id,
                    "Generic adapter instance removed"
                );
            }
        }
    }

    /// Sync running instances with config
    pub async fn sync_with_config(&self, old_config: &Config, new_config: &Config) {
        let old_creds = &old_config.credentials;
        let new_creds = &new_config.credentials;

        // Find credentials to stop (removed or deactivated)
        for (id, old_cred) in old_creds {
            match new_creds.get(id) {
                None => {
                    // Credential removed
                    tracing::info!(credential_id = %id, "Credential removed, stopping instance");
                    self.stop_task(id).await;
                }
                Some(new_cred) if !new_cred.active && old_cred.active => {
                    // Credential deactivated
                    tracing::info!(credential_id = %id, "Credential deactivated, stopping instance");
                    self.stop_task(id).await;
                }
                Some(new_cred) if credential_changed(old_cred, new_cred) => {
                    // Credential config changed, restart
                    tracing::info!(credential_id = %id, "Credential config changed, restarting instance");
                    self.stop_task(id).await;
                    if new_cred.active {
                        self.spawn_task(id.clone(), new_cred.clone()).await;
                    }
                }
                _ => {}
            }
        }

        // Find credentials to start (new or activated)
        for (id, new_cred) in new_creds {
            if new_cred.active && !old_creds.contains_key(id) {
                // New credential
                tracing::info!(credential_id = %id, "New credential, starting instance");
                self.spawn_task(id.clone(), new_cred.clone()).await;
            } else if let Some(old_cred) = old_creds.get(id) {
                if new_cred.active && !old_cred.active {
                    // Credential activated
                    tracing::info!(credential_id = %id, "Credential activated, starting instance");
                    self.spawn_task(id.clone(), new_cred.clone()).await;
                }
            }
        }
    }

    /// Stop all instances (for graceful shutdown)
    pub async fn shutdown(&self) {
        tracing::info!("Shutting down all adapter instances");
        let instances = self.registry.instances.read().await;
        let ids: Vec<_> = instances.keys().cloned().collect();
        drop(instances);

        for id in ids {
            self.stop_task(&id).await;
        }
    }
}

/// Check if credential config has changed in a way that requires instance restart
fn credential_changed(old: &CredentialConfig, new: &CredentialConfig) -> bool {
    old.adapter != new.adapter
        || old.token != new.token
        || old.config != new.config
}
