use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub gateway: GatewayConfig,
    pub auth: AuthConfig,
    #[serde(default)]
    pub health_checks: HashMap<String, HealthCheckConfig>,
    #[serde(default)]
    pub credentials: HashMap<String, CredentialConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    pub listen: String,
    pub admin_token: String,
    pub default_target: TargetConfig,
    /// Directory containing adapter definitions
    #[serde(default = "default_adapters_dir")]
    pub adapters_dir: String,
    /// Port range for adapter processes [start, end]
    #[serde(default = "default_adapter_port_range")]
    pub adapter_port_range: (u16, u16),
    #[serde(default)]
    pub file_cache: Option<FileCacheConfig>,
}

fn default_adapters_dir() -> String {
    "./adapters".to_string()
}

fn default_adapter_port_range() -> (u16, u16) {
    (9000, 9100)
}

/// Backend protocol type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendProtocol {
    /// Pipelit: POST webhook, callback via /api/v1/send
    Pipelit,
    /// OpenCode: REST + SSE polling
    Opencode,
}

/// Backend target configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetConfig {
    pub protocol: BackendProtocol,
    /// Inbound URL for Pipelit (POST destination)
    #[serde(default)]
    pub inbound_url: Option<String>,
    /// Base URL for OpenCode
    #[serde(default)]
    pub base_url: Option<String>,
    /// Auth token for the backend
    pub token: String,
    /// Poll interval for OpenCode (milliseconds)
    #[serde(default)]
    pub poll_interval_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileCacheConfig {
    pub directory: String,
    pub ttl_hours: u32,
    pub max_cache_size_mb: u32,
    pub cleanup_interval_minutes: u32,
    pub max_file_size_mb: u32,
    #[serde(default)]
    pub allowed_mime_types: Vec<String>,
    #[serde(default)]
    pub blocked_mime_types: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub send_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckConfig {
    pub url: String,
    pub interval_seconds: u32,
    pub alert_after_failures: u32,
    #[serde(default)]
    pub notify_credentials: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialConfig {
    /// Adapter name (must exist in adapters_dir, or "generic" for built-in)
    pub adapter: String,
    /// Protocol auth token (passed to adapter via env var)
    pub token: String,
    pub active: bool,
    #[serde(default)]
    pub emergency: bool,
    /// Adapter-specific configuration (passed to adapter as JSON)
    #[serde(default)]
    pub config: Option<serde_json::Value>,
    /// Per-credential backend target override. If None, uses gateway.default_target
    #[serde(default)]
    pub target: Option<TargetConfig>,
    /// Opaque routing info passed to backend
    pub route: serde_json::Value,
}

/// Load config from file, resolving environment variables
pub fn load_config<P: AsRef<Path>>(path: P) -> Result<Config, AppError> {
    let content = fs::read_to_string(path)
        .map_err(|e| AppError::Config(format!("Failed to read config file: {}", e)))?;

    // Resolve environment variables in the content
    let resolved = resolve_env_vars(&content)?;

    let config: Config = serde_json::from_str(&resolved)
        .map_err(|e| AppError::Config(format!("Failed to parse config: {}", e)))?;

    Ok(config)
}

/// Resolve ${VAR} patterns to environment variable values
fn resolve_env_vars(content: &str) -> Result<String, AppError> {
    let mut result = content.to_string();
    let re = regex::Regex::new(r"\$\{([^}]+)\}").unwrap();

    for cap in re.captures_iter(content) {
        let var_name = &cap[1];
        let var_value = std::env::var(var_name)
            .map_err(|_| AppError::Config(format!("Environment variable not set: {}", var_name)))?;
        result = result.replace(&cap[0], &var_value);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_env_vars() {
        std::env::set_var("TEST_VAR", "test_value");
        let input = r#"{"token": "${TEST_VAR}"}"#;
        let result = resolve_env_vars(input).unwrap();
        assert_eq!(result, r#"{"token": "test_value"}"#);
    }
}
