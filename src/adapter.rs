//! External Adapter Management
//!
//! Handles discovery, spawning, and communication with external adapter processes.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::sync::RwLock;

use crate::error::AppError;

/// HTTP client for adapter health checks
static HEALTH_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();

fn get_health_client() -> &'static reqwest::Client {
    HEALTH_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("Failed to create HTTP client")
    })
}

/// Adapter definition from adapter.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterDef {
    pub name: String,
    pub version: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// Load adapter definition from a directory
pub fn load_adapter_def(adapter_dir: &Path) -> Result<AdapterDef, AppError> {
    let adapter_json = adapter_dir.join("adapter.json");
    let content = std::fs::read_to_string(&adapter_json).map_err(|e| {
        AppError::Config(format!("Failed to read {}: {}", adapter_json.display(), e))
    })?;

    serde_json::from_str(&content)
        .map_err(|e| AppError::Config(format!("Failed to parse {}: {}", adapter_json.display(), e)))
}

/// Discover all adapters in the adapters directory
pub fn discover_adapters(adapters_dir: &Path) -> Result<HashMap<String, AdapterDef>, AppError> {
    let mut adapters = HashMap::new();

    if !adapters_dir.exists() {
        tracing::warn!(
            path = %adapters_dir.display(),
            "Adapters directory does not exist"
        );
        return Ok(adapters);
    }

    let entries = std::fs::read_dir(adapters_dir).map_err(|e| {
        AppError::Config(format!(
            "Failed to read adapters directory {}: {}",
            adapters_dir.display(),
            e
        ))
    })?;

    for entry in entries {
        let entry = entry
            .map_err(|e| AppError::Config(format!("Failed to read directory entry: {}", e)))?;

        let path = entry.path();
        if path.is_dir() {
            let adapter_json = path.join("adapter.json");
            if adapter_json.exists() {
                match load_adapter_def(&path) {
                    Ok(def) => {
                        tracing::info!(
                            adapter = %def.name,
                            version = %def.version,
                            "Discovered adapter"
                        );
                        adapters.insert(def.name.clone(), def);
                    }
                    Err(e) => {
                        tracing::warn!(
                            path = %path.display(),
                            error = %e,
                            "Failed to load adapter definition"
                        );
                    }
                }
            }
        }
    }

    Ok(adapters)
}

/// Adapter health state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterHealth {
    /// Initial state after spawn, waiting for health check
    Starting,
    /// Adapter is healthy and responding
    Healthy,
    /// Adapter failed to respond to health check
    Unhealthy,
    /// Adapter process has exited
    Dead,
}

/// Running adapter process info
#[allow(dead_code)]
pub struct AdapterProcess {
    pub instance_id: String,
    pub credential_id: String,
    pub adapter_name: String,
    pub port: u16,
    pub process: Child,
    pub health: AdapterHealth,
    pub consecutive_failures: u32,
    pub restart_count: u32,
    pub last_restart: Option<std::time::Instant>,
    /// Stored for restart
    pub token: String,
    pub config: Option<serde_json::Value>,
}

/// Port allocator for adapter processes
pub struct PortAllocator {
    range_start: u16,
    range_end: u16,
    allocated: RwLock<Vec<u16>>,
}

impl PortAllocator {
    pub fn new(range: (u16, u16)) -> Self {
        Self {
            range_start: range.0,
            range_end: range.1,
            allocated: RwLock::new(Vec::new()),
        }
    }

    pub async fn allocate(&self) -> Option<u16> {
        let mut allocated = self.allocated.write().await;
        for port in self.range_start..=self.range_end {
            if !allocated.contains(&port) {
                allocated.push(port);
                return Some(port);
            }
        }
        None
    }

    pub async fn release(&self, port: u16) {
        let mut allocated = self.allocated.write().await;
        allocated.retain(|&p| p != port);
    }
}

/// Adapter Instance Manager
pub struct AdapterInstanceManager {
    /// Discovered adapter definitions
    pub adapters: HashMap<String, AdapterDef>,
    /// Adapters directory path
    pub adapters_dir: String,
    /// Running adapter processes (credential_id -> process info)
    processes: RwLock<HashMap<String, AdapterProcess>>,
    /// Port allocator
    port_allocator: PortAllocator,
    /// Gateway URL for adapters to call back
    gateway_url: String,
}

impl AdapterInstanceManager {
    pub fn new(
        adapters_dir: String,
        port_range: (u16, u16),
        gateway_listen: &str,
    ) -> Result<Self, AppError> {
        let adapters_path = Path::new(&adapters_dir);
        let adapters = discover_adapters(adapters_path)?;

        // Construct gateway URL from listen address
        let gateway_url = if gateway_listen.starts_with("0.0.0.0") {
            format!(
                "http://127.0.0.1:{}",
                gateway_listen.split(':').next_back().unwrap_or("8080")
            )
        } else {
            format!("http://{}", gateway_listen)
        };

        Ok(Self {
            adapters,
            adapters_dir,
            processes: RwLock::new(HashMap::new()),
            port_allocator: PortAllocator::new(port_range),
            gateway_url,
        })
    }

    /// Check if an adapter exists
    #[allow(dead_code)]
    pub fn has_adapter(&self, name: &str) -> bool {
        name == "generic" || self.adapters.contains_key(name)
    }

    /// Spawn an adapter process for a credential
    pub async fn spawn(
        &self,
        credential_id: &str,
        adapter_name: &str,
        token: &str,
        config: Option<&serde_json::Value>,
    ) -> Result<(String, u16), AppError> {
        // Generic adapter is built-in, no process to spawn
        if adapter_name == "generic" {
            let instance_id = format!("generic_{}", uuid::Uuid::new_v4());
            return Ok((instance_id, 0));
        }

        // Get adapter definition
        let adapter_def = self
            .adapters
            .get(adapter_name)
            .ok_or_else(|| AppError::Config(format!("Adapter not found: {}", adapter_name)))?;

        // Allocate port
        let port = self
            .port_allocator
            .allocate()
            .await
            .ok_or_else(|| AppError::Internal("No available ports for adapter".to_string()))?;

        let instance_id = format!("{}_{}", adapter_name, uuid::Uuid::new_v4());

        // Build command
        let adapter_path = Path::new(&self.adapters_dir).join(adapter_name);
        let mut cmd = Command::new(&adapter_def.command);

        cmd.args(&adapter_def.args)
            .current_dir(&adapter_path)
            .env("INSTANCE_ID", &instance_id)
            .env("ADAPTER_PORT", port.to_string())
            .env("GATEWAY_URL", &self.gateway_url)
            .env("CREDENTIAL_ID", credential_id)
            .env("CREDENTIAL_TOKEN", token)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(cfg) = config {
            cmd.env(
                "CREDENTIAL_CONFIG",
                serde_json::to_string(cfg).unwrap_or_default(),
            );
        }

        tracing::info!(
            credential_id = %credential_id,
            adapter = %adapter_name,
            port = %port,
            instance_id = %instance_id,
            "Spawning adapter process"
        );

        let process = cmd
            .spawn()
            .map_err(|e| AppError::Internal(format!("Failed to spawn adapter process: {}", e)))?;

        // Store process info
        let mut processes = self.processes.write().await;
        processes.insert(
            credential_id.to_string(),
            AdapterProcess {
                instance_id: instance_id.clone(),
                credential_id: credential_id.to_string(),
                adapter_name: adapter_name.to_string(),
                port,
                process,
                health: AdapterHealth::Starting,
                consecutive_failures: 0,
                restart_count: 0,
                last_restart: None,
                token: token.to_string(),
                config: config.cloned(),
            },
        );

        Ok((instance_id, port))
    }

    /// Stop an adapter process
    pub async fn stop(&self, credential_id: &str) -> Result<(), AppError> {
        let mut processes = self.processes.write().await;

        if let Some(mut process_info) = processes.remove(credential_id) {
            // Release port
            if process_info.port > 0 {
                self.port_allocator.release(process_info.port).await;
            }

            // Kill process
            if let Err(e) = process_info.process.kill().await {
                tracing::warn!(
                    credential_id = %credential_id,
                    error = %e,
                    "Failed to kill adapter process (may have already exited)"
                );
            }

            tracing::info!(
                credential_id = %credential_id,
                adapter = %process_info.adapter_name,
                "Adapter process stopped"
            );
        }

        Ok(())
    }

    /// Get the port for a credential's adapter
    pub async fn get_port(&self, credential_id: &str) -> Option<u16> {
        let processes = self.processes.read().await;
        processes.get(credential_id).map(|p| p.port)
    }

    /// Get instance_id for a credential
    #[allow(dead_code)]
    pub async fn get_instance_id(&self, credential_id: &str) -> Option<String> {
        let processes = self.processes.read().await;
        processes.get(credential_id).map(|p| p.instance_id.clone())
    }

    /// Check if adapter process is running for a credential
    #[allow(dead_code)]
    pub async fn is_running(&self, credential_id: &str) -> bool {
        let processes = self.processes.read().await;
        processes.contains_key(credential_id)
    }

    /// Stop all adapter processes
    pub async fn stop_all(&self) {
        let processes = self.processes.read().await;
        let ids: Vec<_> = processes.keys().cloned().collect();
        drop(processes);

        for id in ids {
            if let Err(e) = self.stop(&id).await {
                tracing::error!(
                    credential_id = %id,
                    error = %e,
                    "Failed to stop adapter"
                );
            }
        }
    }

    /// Find credential_id by instance_id
    pub async fn get_credential_id(&self, instance_id: &str) -> Option<String> {
        let processes = self.processes.read().await;
        for (cred_id, process) in processes.iter() {
            if process.instance_id == instance_id {
                return Some(cred_id.clone());
            }
        }
        None
    }

    /// Check health of a specific adapter by credential_id
    pub async fn check_health(&self, credential_id: &str) -> AdapterHealth {
        let port = {
            let processes = self.processes.read().await;
            match processes.get(credential_id) {
                Some(p) if p.port > 0 => p.port,
                _ => return AdapterHealth::Dead, // Generic adapter or not found
            }
        };

        let client = get_health_client();
        let url = format!("http://127.0.0.1:{}/health", port);

        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => AdapterHealth::Healthy,
            Ok(resp) => {
                tracing::warn!(
                    credential_id = %credential_id,
                    status = %resp.status(),
                    "Adapter health check returned non-success status"
                );
                AdapterHealth::Unhealthy
            }
            Err(e) => {
                tracing::warn!(
                    credential_id = %credential_id,
                    error = %e,
                    "Adapter health check failed"
                );
                AdapterHealth::Unhealthy
            }
        }
    }

    /// Update health state for a credential
    pub async fn update_health(
        &self,
        credential_id: &str,
        health: AdapterHealth,
        reset_failures: bool,
    ) {
        let mut processes = self.processes.write().await;
        if let Some(process) = processes.get_mut(credential_id) {
            process.health = health;
            if reset_failures {
                process.consecutive_failures = 0;
            } else if health == AdapterHealth::Unhealthy {
                process.consecutive_failures += 1;
            }
        }
    }

    /// Get health state for a credential
    #[allow(dead_code)]
    pub async fn get_health(&self, credential_id: &str) -> Option<(AdapterHealth, u32)> {
        let processes = self.processes.read().await;
        processes
            .get(credential_id)
            .map(|p| (p.health, p.consecutive_failures))
    }

    /// Check if adapter process has exited
    pub async fn check_process_alive(&self, credential_id: &str) -> bool {
        let mut processes = self.processes.write().await;
        if let Some(process) = processes.get_mut(credential_id) {
            if process.port == 0 {
                return true; // Generic adapter, always "alive"
            }
            match process.process.try_wait() {
                Ok(Some(status)) => {
                    tracing::warn!(
                        credential_id = %credential_id,
                        status = ?status,
                        "Adapter process has exited"
                    );
                    process.health = AdapterHealth::Dead;
                    false
                }
                Ok(None) => true, // Still running
                Err(e) => {
                    tracing::error!(
                        credential_id = %credential_id,
                        error = %e,
                        "Failed to check process status"
                    );
                    false
                }
            }
        } else {
            false
        }
    }

    /// Get all credentials with their health status
    pub async fn get_all_health(&self) -> HashMap<String, (String, AdapterHealth, u32)> {
        let processes = self.processes.read().await;
        processes
            .iter()
            .map(|(cred_id, p)| {
                (
                    cred_id.clone(),
                    (p.adapter_name.clone(), p.health, p.consecutive_failures),
                )
            })
            .collect()
    }

    /// Restart an adapter process
    /// Returns Ok(true) if restart succeeded, Ok(false) if should wait (backoff), Err on failure
    pub async fn restart(&self, credential_id: &str, max_restarts: u32) -> Result<bool, AppError> {
        // Get info needed for restart
        let (adapter_name, token, config, restart_count, last_restart, _old_port) = {
            let processes = self.processes.read().await;
            let process = processes.get(credential_id).ok_or_else(|| {
                AppError::Internal(format!(
                    "Process not found for credential: {}",
                    credential_id
                ))
            })?;

            (
                process.adapter_name.clone(),
                process.token.clone(),
                process.config.clone(),
                process.restart_count,
                process.last_restart,
                process.port,
            )
        };

        // Check if we've exceeded max restarts
        if restart_count >= max_restarts {
            tracing::error!(
                credential_id = %credential_id,
                restart_count = %restart_count,
                max_restarts = %max_restarts,
                "Max restarts exceeded, not restarting"
            );
            return Err(AppError::Internal("Max restarts exceeded".to_string()));
        }

        // Calculate backoff delay (exponential: 1s, 2s, 4s, 8s, etc. up to 60s)
        let backoff_secs = std::cmp::min(60, 1u64 << restart_count);
        let backoff = Duration::from_secs(backoff_secs);

        // Check if we need to wait
        if let Some(last) = last_restart {
            let elapsed = last.elapsed();
            if elapsed < backoff {
                let remaining = backoff - elapsed;
                tracing::info!(
                    credential_id = %credential_id,
                    remaining_secs = remaining.as_secs(),
                    "Backoff in effect, waiting before restart"
                );
                return Ok(false);
            }
        }

        tracing::info!(
            credential_id = %credential_id,
            adapter = %adapter_name,
            restart_count = restart_count + 1,
            "Restarting adapter"
        );

        // Stop the old process first
        self.stop(credential_id).await?;

        // Respawn with same settings
        let result = self
            .spawn(credential_id, &adapter_name, &token, config.as_ref())
            .await;

        // Update restart tracking
        if result.is_ok() {
            let mut processes = self.processes.write().await;
            if let Some(process) = processes.get_mut(credential_id) {
                process.restart_count = restart_count + 1;
                process.last_restart = Some(std::time::Instant::now());
            }
        }

        result.map(|_| true)
    }

    /// Reset restart count for a credential (called when adapter is healthy for a while)
    pub async fn reset_restart_count(&self, credential_id: &str) {
        let mut processes = self.processes.write().await;
        if let Some(process) = processes.get_mut(credential_id)
            && process.restart_count > 0
        {
            tracing::debug!(
                credential_id = %credential_id,
                old_count = %process.restart_count,
                "Resetting restart count"
            );
            process.restart_count = 0;
        }
    }

    /// Get restart info for a credential
    #[allow(dead_code)]
    pub async fn get_restart_info(&self, credential_id: &str) -> Option<(u32, Option<Duration>)> {
        let processes = self.processes.read().await;
        processes.get(credential_id).map(|p| {
            let time_since_restart = p.last_restart.map(|t| t.elapsed());
            (p.restart_count, time_since_restart)
        })
    }
}

/// Configuration for adapter health monitor
pub struct HealthMonitorConfig {
    /// How often to check adapter health (seconds)
    pub interval_secs: u64,
    /// Number of consecutive failures before restart
    pub max_failures: u32,
    /// Maximum number of restart attempts
    pub max_restarts: u32,
    /// How long an adapter must be healthy before resetting restart count (seconds)
    pub healthy_reset_secs: u64,
}

impl Default for HealthMonitorConfig {
    fn default() -> Self {
        Self {
            interval_secs: 30,
            max_failures: 3,
            max_restarts: 5,
            healthy_reset_secs: 300, // 5 minutes
        }
    }
}

/// Start health monitoring for all adapters
/// This runs in a background task and periodically checks adapter health
pub async fn start_adapter_health_monitor(
    manager: Arc<AdapterInstanceManager>,
    interval_secs: u64,
    max_failures: u32,
) {
    let config = HealthMonitorConfig {
        interval_secs,
        max_failures,
        ..Default::default()
    };

    start_adapter_health_monitor_with_config(manager, config).await;
}

/// Start health monitoring with full configuration
pub async fn start_adapter_health_monitor_with_config(
    manager: Arc<AdapterInstanceManager>,
    config: HealthMonitorConfig,
) {
    let interval = Duration::from_secs(config.interval_secs);
    let healthy_reset = Duration::from_secs(config.healthy_reset_secs);

    tracing::info!(
        interval_secs = %config.interval_secs,
        max_failures = %config.max_failures,
        max_restarts = %config.max_restarts,
        "Starting adapter health monitor"
    );

    // Track how long each adapter has been healthy
    let mut healthy_since: HashMap<String, std::time::Instant> = HashMap::new();

    loop {
        tokio::time::sleep(interval).await;

        let health_status = manager.get_all_health().await;

        for (credential_id, (adapter_name, current_health, consecutive_failures)) in health_status {
            // Skip generic adapter (built-in)
            if adapter_name == "generic" {
                continue;
            }

            // First check if process is still running
            if !manager.check_process_alive(&credential_id).await {
                tracing::warn!(
                    credential_id = %credential_id,
                    adapter = %adapter_name,
                    "Adapter process died, attempting restart"
                );
                healthy_since.remove(&credential_id);

                match manager.restart(&credential_id, config.max_restarts).await {
                    Ok(true) => {
                        tracing::info!(
                            credential_id = %credential_id,
                            "Adapter restart initiated"
                        );
                        // Wait for adapter to become ready
                        let ready = wait_for_adapter_ready(
                            &manager,
                            &credential_id,
                            Duration::from_secs(30),
                            Duration::from_millis(500),
                        )
                        .await;
                        if ready {
                            tracing::info!(
                                credential_id = %credential_id,
                                "Restarted adapter is ready"
                            );
                        }
                    }
                    Ok(false) => {
                        tracing::debug!(
                            credential_id = %credential_id,
                            "Restart postponed due to backoff"
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            credential_id = %credential_id,
                            error = %e,
                            "Failed to restart adapter"
                        );
                    }
                }
                continue;
            }

            // Run health check
            let health = manager.check_health(&credential_id).await;

            match health {
                AdapterHealth::Healthy => {
                    if current_health != AdapterHealth::Healthy {
                        tracing::info!(
                            credential_id = %credential_id,
                            adapter = %adapter_name,
                            "Adapter is now healthy"
                        );
                        healthy_since.insert(credential_id.clone(), std::time::Instant::now());
                    }
                    manager
                        .update_health(&credential_id, AdapterHealth::Healthy, true)
                        .await;

                    // Check if we should reset restart count
                    if let Some(since) = healthy_since.get(&credential_id) {
                        if since.elapsed() >= healthy_reset {
                            manager.reset_restart_count(&credential_id).await;
                            // Reset the timer
                            healthy_since.insert(credential_id.clone(), std::time::Instant::now());
                        }
                    } else {
                        healthy_since.insert(credential_id.clone(), std::time::Instant::now());
                    }
                }
                AdapterHealth::Unhealthy => {
                    healthy_since.remove(&credential_id);
                    manager
                        .update_health(&credential_id, AdapterHealth::Unhealthy, false)
                        .await;

                    let new_failures = consecutive_failures + 1;
                    if new_failures >= config.max_failures {
                        tracing::warn!(
                            credential_id = %credential_id,
                            adapter = %adapter_name,
                            consecutive_failures = %new_failures,
                            "Adapter exceeded max failures, attempting restart"
                        );

                        match manager.restart(&credential_id, config.max_restarts).await {
                            Ok(true) => {
                                tracing::info!(
                                    credential_id = %credential_id,
                                    "Adapter restart initiated due to health failures"
                                );
                                // Wait for adapter to become ready
                                let ready = wait_for_adapter_ready(
                                    &manager,
                                    &credential_id,
                                    Duration::from_secs(30),
                                    Duration::from_millis(500),
                                )
                                .await;
                                if ready {
                                    tracing::info!(
                                        credential_id = %credential_id,
                                        "Restarted adapter is ready"
                                    );
                                }
                            }
                            Ok(false) => {
                                tracing::debug!(
                                    credential_id = %credential_id,
                                    "Restart postponed due to backoff"
                                );
                            }
                            Err(e) => {
                                tracing::error!(
                                    credential_id = %credential_id,
                                    error = %e,
                                    "Failed to restart adapter"
                                );
                            }
                        }
                    } else {
                        tracing::warn!(
                            credential_id = %credential_id,
                            adapter = %adapter_name,
                            consecutive_failures = %new_failures,
                            "Adapter health check failed"
                        );
                    }
                }
                _ => {}
            }
        }
    }
}

/// Wait for an adapter to become healthy after spawn
pub async fn wait_for_adapter_ready(
    manager: &AdapterInstanceManager,
    credential_id: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> bool {
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        let health = manager.check_health(credential_id).await;
        if health == AdapterHealth::Healthy {
            manager
                .update_health(credential_id, AdapterHealth::Healthy, true)
                .await;
            return true;
        }
        tokio::time::sleep(poll_interval).await;
    }

    tracing::warn!(
        credential_id = %credential_id,
        timeout_secs = timeout.as_secs(),
        "Adapter did not become ready in time"
    );
    false
}

/// Request body for adapter inbound messages
#[derive(Debug, Deserialize)]
pub struct AdapterInboundRequest {
    pub instance_id: String,
    pub chat_id: String,
    pub message_id: String,
    pub text: String,
    pub from: AdapterUser,
    #[serde(default)]
    pub file: Option<AdapterFileInfo>,
    #[serde(default)]
    pub timestamp: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AdapterUser {
    pub id: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AdapterFileInfo {
    pub url: String,
    #[serde(default)]
    pub auth_header: Option<String>,
    pub filename: String,
    pub mime_type: String,
}

/// Request body for sending to adapter
#[derive(Debug, Serialize)]
pub struct AdapterSendRequest {
    pub chat_id: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
}

/// Response from adapter send
#[derive(Debug, Deserialize)]
pub struct AdapterSendResponse {
    pub protocol_message_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_port_allocator_basic() {
        let allocator = PortAllocator::new((9000, 9002));

        // Allocate first port
        let port1 = allocator.allocate().await;
        assert_eq!(port1, Some(9000));

        // Allocate second port
        let port2 = allocator.allocate().await;
        assert_eq!(port2, Some(9001));

        // Allocate third port
        let port3 = allocator.allocate().await;
        assert_eq!(port3, Some(9002));

        // No more ports available
        let port4 = allocator.allocate().await;
        assert_eq!(port4, None);
    }

    #[tokio::test]
    async fn test_port_allocator_release() {
        let allocator = PortAllocator::new((9000, 9001));

        // Allocate all ports
        let port1 = allocator.allocate().await.unwrap();
        let _port2 = allocator.allocate().await.unwrap();
        assert!(allocator.allocate().await.is_none());

        // Release first port
        allocator.release(port1).await;

        // Should be able to allocate again
        let port3 = allocator.allocate().await;
        assert_eq!(port3, Some(9000));
    }

    #[test]
    fn test_adapter_def_parse() {
        let json = r#"{
            "name": "telegram",
            "version": "1.0.0",
            "command": "python3",
            "args": ["main.py"]
        }"#;

        let def: AdapterDef = serde_json::from_str(json).unwrap();
        assert_eq!(def.name, "telegram");
        assert_eq!(def.version, "1.0.0");
        assert_eq!(def.command, "python3");
        assert_eq!(def.args, vec!["main.py"]);
    }

    #[test]
    fn test_adapter_def_parse_minimal() {
        let json = r#"{
            "name": "test",
            "version": "0.1.0",
            "command": "node"
        }"#;

        let def: AdapterDef = serde_json::from_str(json).unwrap();
        assert_eq!(def.name, "test");
        assert!(def.args.is_empty());
    }

    #[test]
    fn test_health_monitor_config_default() {
        let config = HealthMonitorConfig::default();
        assert_eq!(config.interval_secs, 30);
        assert_eq!(config.max_failures, 3);
        assert_eq!(config.max_restarts, 5);
        assert_eq!(config.healthy_reset_secs, 300);
    }

    #[test]
    fn test_adapter_inbound_request_parse() {
        let json = r#"{
            "instance_id": "telegram_abc123",
            "chat_id": "12345",
            "message_id": "msg_001",
            "from": {
                "id": "user_1",
                "username": "testuser",
                "display_name": "Test User"
            },
            "text": "Hello, world!"
        }"#;

        let req: AdapterInboundRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.instance_id, "telegram_abc123");
        assert_eq!(req.chat_id, "12345");
        assert_eq!(req.text, "Hello, world!");
        assert_eq!(req.from.id, "user_1");
        assert_eq!(req.from.username, Some("testuser".to_string()));
    }

    #[test]
    fn test_adapter_send_request_serialize() {
        let req = AdapterSendRequest {
            chat_id: "12345".to_string(),
            text: "Hello!".to_string(),
            reply_to_message_id: None,
            file_path: None,
        };

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"chat_id\":\"12345\""));
        assert!(json.contains("\"text\":\"Hello!\""));
        // Optional fields should be skipped
        assert!(!json.contains("reply_to_message_id"));
        assert!(!json.contains("file_path"));
    }

    #[test]
    fn test_adapter_send_request_with_reply() {
        let req = AdapterSendRequest {
            chat_id: "12345".to_string(),
            text: "Reply!".to_string(),
            reply_to_message_id: Some("msg_001".to_string()),
            file_path: None,
        };

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"reply_to_message_id\":\"msg_001\""));
    }
}
