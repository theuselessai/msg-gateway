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
#[allow(dead_code)]
pub enum InstanceStatus {
    Starting,
    Running,
    Stopping,
    Stopped,
    Error(String),
}

/// Info about a running adapter instance
#[allow(dead_code)]
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
    instances: RwLock<HashMap<String, InstanceInfo>>, // credential_id -> instance
}

impl Default for TaskRegistry {
    fn default() -> Self {
        Self::new()
    }
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
    #[allow(dead_code)]
    pub async fn get_instance(&self, credential_id: &str) -> Option<(String, u16)> {
        let instances = self.instances.read().await;
        instances
            .get(credential_id)
            .map(|info| (info.instance_id.clone(), info.port))
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
    #[allow(dead_code)]
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

impl Default for CredentialManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialManager {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(TaskRegistry::new()),
        }
    }

    /// Spawn instances for all active credentials in config
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
            } else if let Some(old_cred) = old_creds.get(id)
                && new_cred.active
                && !old_cred.active
            {
                // Credential activated
                tracing::info!(credential_id = %id, "Credential activated, starting instance");
                self.spawn_task(id.clone(), new_cred.clone()).await;
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
#[allow(dead_code)]
fn credential_changed(old: &CredentialConfig, new: &CredentialConfig) -> bool {
    old.adapter != new.adapter || old.token != new.token || old.config != new.config
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AuthConfig, BackendProtocol, GatewayConfig, TargetConfig};

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
                },
                adapters_dir: "./adapters".to_string(),
                adapter_port_range: (9000, 9100),
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

    // ==================== TaskRegistry Tests ====================

    #[tokio::test]
    async fn test_task_registry_new() {
        let registry = TaskRegistry::new();
        let status = registry.get_all_status().await;
        assert!(status.is_empty());
    }

    #[tokio::test]
    async fn test_task_registry_default() {
        let registry = TaskRegistry::default();
        let status = registry.get_all_status().await;
        assert!(status.is_empty());
    }

    #[tokio::test]
    async fn test_task_registry_register() {
        let registry = TaskRegistry::new();
        registry
            .register(
                "cred1".to_string(),
                "instance_123".to_string(),
                "generic".to_string(),
                0,
            )
            .await;

        assert!(registry.is_running("cred1").await);
        assert_eq!(
            registry.get_status("cred1").await,
            Some(InstanceStatus::Running)
        );
    }

    #[tokio::test]
    async fn test_task_registry_get_instance() {
        let registry = TaskRegistry::new();
        registry
            .register(
                "cred1".to_string(),
                "instance_123".to_string(),
                "telegram".to_string(),
                9001,
            )
            .await;

        let instance = registry.get_instance("cred1").await;
        assert_eq!(instance, Some(("instance_123".to_string(), 9001)));
    }

    #[tokio::test]
    async fn test_task_registry_get_instance_not_found() {
        let registry = TaskRegistry::new();
        let instance = registry.get_instance("nonexistent").await;
        assert_eq!(instance, None);
    }

    #[tokio::test]
    async fn test_task_registry_remove() {
        let registry = TaskRegistry::new();
        registry
            .register(
                "cred1".to_string(),
                "instance_123".to_string(),
                "generic".to_string(),
                0,
            )
            .await;

        let removed = registry.remove("cred1").await;
        assert!(removed.is_some());
        assert!(!registry.is_running("cred1").await);
    }

    #[tokio::test]
    async fn test_task_registry_remove_nonexistent() {
        let registry = TaskRegistry::new();
        let removed = registry.remove("nonexistent").await;
        assert!(removed.is_none());
    }

    #[tokio::test]
    async fn test_task_registry_update_status() {
        let registry = TaskRegistry::new();
        registry
            .register(
                "cred1".to_string(),
                "instance_123".to_string(),
                "generic".to_string(),
                0,
            )
            .await;

        registry
            .update_status("cred1", InstanceStatus::Error("test error".to_string()))
            .await;

        let status = registry.get_status("cred1").await;
        assert_eq!(
            status,
            Some(InstanceStatus::Error("test error".to_string()))
        );
        // is_running should return false for Error status
        assert!(!registry.is_running("cred1").await);
    }

    #[tokio::test]
    async fn test_task_registry_update_status_nonexistent() {
        let registry = TaskRegistry::new();
        // Should not panic
        registry
            .update_status("nonexistent", InstanceStatus::Stopped)
            .await;
    }

    #[tokio::test]
    async fn test_task_registry_get_all_status() {
        let registry = TaskRegistry::new();
        registry
            .register(
                "cred1".to_string(),
                "instance_1".to_string(),
                "generic".to_string(),
                0,
            )
            .await;
        registry
            .register(
                "cred2".to_string(),
                "instance_2".to_string(),
                "telegram".to_string(),
                9001,
            )
            .await;

        let all_status = registry.get_all_status().await;
        assert_eq!(all_status.len(), 2);
        assert_eq!(
            all_status.get("cred1"),
            Some(&("generic".to_string(), InstanceStatus::Running))
        );
        assert_eq!(
            all_status.get("cred2"),
            Some(&("telegram".to_string(), InstanceStatus::Running))
        );
    }

    #[tokio::test]
    async fn test_task_registry_get_status_nonexistent() {
        let registry = TaskRegistry::new();
        assert_eq!(registry.get_status("nonexistent").await, None);
    }

    #[tokio::test]
    async fn test_task_registry_is_running_false_for_stopped() {
        let registry = TaskRegistry::new();
        registry
            .register(
                "cred1".to_string(),
                "instance_123".to_string(),
                "generic".to_string(),
                0,
            )
            .await;
        registry
            .update_status("cred1", InstanceStatus::Stopped)
            .await;

        assert!(!registry.is_running("cred1").await);
    }

    // ==================== CredentialManager Tests ====================

    #[tokio::test]
    async fn test_credential_manager_new() {
        let manager = CredentialManager::new();
        let status = manager.registry.get_all_status().await;
        assert!(status.is_empty());
    }

    #[tokio::test]
    async fn test_credential_manager_default() {
        let manager = CredentialManager::default();
        let status = manager.registry.get_all_status().await;
        assert!(status.is_empty());
    }

    #[tokio::test]
    async fn test_credential_manager_spawn_task_generic() {
        let manager = CredentialManager::new();
        let cred = make_credential("generic", "token123", true);

        manager.spawn_task("cred1".to_string(), cred).await;

        assert!(manager.registry.is_running("cred1").await);
        let instance = manager.registry.get_instance("cred1").await;
        assert!(instance.is_some());
        let (instance_id, port) = instance.unwrap();
        assert!(instance_id.starts_with("generic_"));
        assert_eq!(port, 0);
    }

    #[tokio::test]
    async fn test_credential_manager_spawn_task_external() {
        let manager = CredentialManager::new();
        let cred = make_credential("telegram", "token123", true);

        manager.spawn_task("cred1".to_string(), cred).await;

        assert!(manager.registry.is_running("cred1").await);
        let instance = manager.registry.get_instance("cred1").await;
        assert!(instance.is_some());
        let (instance_id, _port) = instance.unwrap();
        assert!(instance_id.starts_with("telegram_"));
    }

    #[tokio::test]
    async fn test_credential_manager_spawn_task_already_running() {
        let manager = CredentialManager::new();
        let cred = make_credential("generic", "token123", true);

        // Spawn first time
        manager.spawn_task("cred1".to_string(), cred.clone()).await;
        let first_instance = manager.registry.get_instance("cred1").await.unwrap();

        // Try to spawn again - should be skipped
        manager.spawn_task("cred1".to_string(), cred).await;
        let second_instance = manager.registry.get_instance("cred1").await.unwrap();

        // Instance ID should be the same (not replaced)
        assert_eq!(first_instance.0, second_instance.0);
    }

    #[tokio::test]
    async fn test_credential_manager_stop_task_generic() {
        let manager = CredentialManager::new();
        let cred = make_credential("generic", "token123", true);

        manager.spawn_task("cred1".to_string(), cred).await;
        assert!(manager.registry.is_running("cred1").await);

        manager.stop_task("cred1").await;
        assert!(!manager.registry.is_running("cred1").await);
    }

    #[tokio::test]
    async fn test_credential_manager_stop_task_external() {
        let manager = CredentialManager::new();
        let cred = make_credential("telegram", "token123", true);

        manager.spawn_task("cred1".to_string(), cred).await;
        assert!(manager.registry.is_running("cred1").await);

        manager.stop_task("cred1").await;
        assert!(!manager.registry.is_running("cred1").await);
    }

    #[tokio::test]
    async fn test_credential_manager_stop_task_nonexistent() {
        let manager = CredentialManager::new();
        // Should not panic
        manager.stop_task("nonexistent").await;
    }

    #[tokio::test]
    async fn test_credential_manager_start_all() {
        let manager = CredentialManager::new();
        let config = make_config(vec![
            ("active1", make_credential("generic", "token1", true)),
            ("active2", make_credential("telegram", "token2", true)),
            ("inactive", make_credential("generic", "token3", false)),
        ]);

        manager.start_all(&config).await;

        assert!(manager.registry.is_running("active1").await);
        assert!(manager.registry.is_running("active2").await);
        assert!(!manager.registry.is_running("inactive").await);
    }

    #[tokio::test]
    async fn test_credential_manager_shutdown() {
        let manager = CredentialManager::new();
        let config = make_config(vec![
            ("cred1", make_credential("generic", "token1", true)),
            ("cred2", make_credential("telegram", "token2", true)),
        ]);

        manager.start_all(&config).await;
        assert!(manager.registry.is_running("cred1").await);
        assert!(manager.registry.is_running("cred2").await);

        manager.shutdown().await;

        assert!(!manager.registry.is_running("cred1").await);
        assert!(!manager.registry.is_running("cred2").await);
    }

    #[tokio::test]
    async fn test_credential_manager_sync_with_config_new_credential() {
        let manager = CredentialManager::new();

        let old_config = make_config(vec![]);
        let new_config = make_config(vec![("cred1", make_credential("generic", "token1", true))]);

        manager.sync_with_config(&old_config, &new_config).await;

        assert!(manager.registry.is_running("cred1").await);
    }

    #[tokio::test]
    async fn test_credential_manager_sync_with_config_removed_credential() {
        let manager = CredentialManager::new();

        let old_config = make_config(vec![("cred1", make_credential("generic", "token1", true))]);
        let new_config = make_config(vec![]);

        // Start initial state
        manager.start_all(&old_config).await;
        assert!(manager.registry.is_running("cred1").await);

        manager.sync_with_config(&old_config, &new_config).await;

        assert!(!manager.registry.is_running("cred1").await);
    }

    #[tokio::test]
    async fn test_credential_manager_sync_with_config_deactivated() {
        let manager = CredentialManager::new();

        let old_config = make_config(vec![("cred1", make_credential("generic", "token1", true))]);
        let new_config = make_config(vec![("cred1", make_credential("generic", "token1", false))]);

        manager.start_all(&old_config).await;
        assert!(manager.registry.is_running("cred1").await);

        manager.sync_with_config(&old_config, &new_config).await;

        assert!(!manager.registry.is_running("cred1").await);
    }

    #[tokio::test]
    async fn test_credential_manager_sync_with_config_activated() {
        let manager = CredentialManager::new();

        let old_config = make_config(vec![("cred1", make_credential("generic", "token1", false))]);
        let new_config = make_config(vec![("cred1", make_credential("generic", "token1", true))]);

        manager.sync_with_config(&old_config, &new_config).await;

        assert!(manager.registry.is_running("cred1").await);
    }

    #[tokio::test]
    async fn test_credential_manager_sync_with_config_token_changed() {
        let manager = CredentialManager::new();

        let old_config = make_config(vec![("cred1", make_credential("generic", "token1", true))]);
        let new_config = make_config(vec![("cred1", make_credential("generic", "token2", true))]);

        manager.start_all(&old_config).await;
        let old_instance = manager.registry.get_instance("cred1").await.unwrap();

        manager.sync_with_config(&old_config, &new_config).await;

        // Should still be running but with new instance
        assert!(manager.registry.is_running("cred1").await);
        let new_instance = manager.registry.get_instance("cred1").await.unwrap();
        // Instance ID should be different (restarted)
        assert_ne!(old_instance.0, new_instance.0);
    }

    #[tokio::test]
    async fn test_credential_manager_sync_with_config_adapter_changed() {
        let manager = CredentialManager::new();

        let old_config = make_config(vec![("cred1", make_credential("generic", "token1", true))]);
        let new_config = make_config(vec![("cred1", make_credential("telegram", "token1", true))]);

        manager.start_all(&old_config).await;

        manager.sync_with_config(&old_config, &new_config).await;

        // Should still be running with new adapter
        assert!(manager.registry.is_running("cred1").await);
        let status = manager.registry.get_all_status().await;
        assert_eq!(status.get("cred1").unwrap().0, "telegram");
    }

    // ==================== credential_changed Tests ====================

    #[test]
    fn test_credential_changed_same() {
        let cred1 = make_credential("generic", "token123", true);
        let cred2 = make_credential("generic", "token123", true);
        assert!(!credential_changed(&cred1, &cred2));
    }

    #[test]
    fn test_credential_changed_adapter() {
        let cred1 = make_credential("generic", "token123", true);
        let cred2 = make_credential("telegram", "token123", true);
        assert!(credential_changed(&cred1, &cred2));
    }

    #[test]
    fn test_credential_changed_token() {
        let cred1 = make_credential("generic", "token1", true);
        let cred2 = make_credential("generic", "token2", true);
        assert!(credential_changed(&cred1, &cred2));
    }

    #[test]
    fn test_credential_changed_active_ignored() {
        // Active status change is handled separately, not by credential_changed
        let cred1 = make_credential("generic", "token123", true);
        let cred2 = make_credential("generic", "token123", false);
        assert!(!credential_changed(&cred1, &cred2));
    }

    #[test]
    fn test_credential_changed_config() {
        let mut cred1 = make_credential("generic", "token123", true);
        let mut cred2 = make_credential("generic", "token123", true);
        cred1.config = Some(serde_json::json!({"key": "value1"}));
        cred2.config = Some(serde_json::json!({"key": "value2"}));
        assert!(credential_changed(&cred1, &cred2));
    }

    #[test]
    fn test_credential_changed_config_none_vs_some() {
        let mut cred1 = make_credential("generic", "token123", true);
        let mut cred2 = make_credential("generic", "token123", true);
        cred1.config = None;
        cred2.config = Some(serde_json::json!({"key": "value"}));
        assert!(credential_changed(&cred1, &cred2));
    }

    // ==================== InstanceStatus Tests ====================

    #[test]
    fn test_instance_status_eq() {
        assert_eq!(InstanceStatus::Running, InstanceStatus::Running);
        assert_eq!(InstanceStatus::Starting, InstanceStatus::Starting);
        assert_eq!(InstanceStatus::Stopping, InstanceStatus::Stopping);
        assert_eq!(InstanceStatus::Stopped, InstanceStatus::Stopped);
        assert_eq!(
            InstanceStatus::Error("test".to_string()),
            InstanceStatus::Error("test".to_string())
        );
        assert_ne!(
            InstanceStatus::Error("test1".to_string()),
            InstanceStatus::Error("test2".to_string())
        );
        assert_ne!(InstanceStatus::Running, InstanceStatus::Stopped);
    }

    #[test]
    fn test_instance_status_clone() {
        let status = InstanceStatus::Error("test error".to_string());
        let cloned = status.clone();
        assert_eq!(status, cloned);
    }

    #[test]
    fn test_instance_status_debug() {
        let status = InstanceStatus::Running;
        let debug_str = format!("{:?}", status);
        assert!(debug_str.contains("Running"));
    }
}
