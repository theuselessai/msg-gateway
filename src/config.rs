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
    /// Directory containing backend adapter definitions
    #[serde(default = "default_backends_dir")]
    pub backends_dir: String,
    /// Port range for backend adapter processes [start, end]
    #[serde(default = "default_backend_port_range")]
    pub backend_port_range: (u16, u16),
    #[serde(default)]
    pub file_cache: Option<FileCacheConfig>,
}

fn default_adapters_dir() -> String {
    "./adapters".to_string()
}

fn default_adapter_port_range() -> (u16, u16) {
    (9000, 9100)
}

fn default_backends_dir() -> String {
    "./backends".to_string()
}

fn default_backend_port_range() -> (u16, u16) {
    (9200, 9300)
}

/// Backend protocol type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendProtocol {
    /// Pipelit: POST webhook, callback via /api/v1/send
    Pipelit,
    /// OpenCode: REST + SSE polling
    Opencode,
    /// External: subprocess-managed backend adapter (any language)
    External,
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
    /// Directory containing the external backend adapter (for External protocol)
    #[serde(default)]
    pub adapter_dir: Option<String>,
    /// Port for pre-spawned external backend adapter
    #[serde(default)]
    pub port: Option<u16>,
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
    use serial_test::serial;
    use tempfile::TempDir;

    // ==================== resolve_env_vars Tests ====================

    #[test]
    #[serial]
    fn test_resolve_env_vars() {
        // SAFETY: This is a single-threaded test
        unsafe {
            std::env::set_var("TEST_VAR", "test_value");
        }
        let input = r#"{"token": "${TEST_VAR}"}"#;
        let result = resolve_env_vars(input).unwrap();
        assert_eq!(result, r#"{"token": "test_value"}"#);
    }

    #[test]
    #[serial]
    fn test_resolve_env_vars_multiple() {
        unsafe {
            std::env::set_var("VAR1", "value1");
            std::env::set_var("VAR2", "value2");
        }
        let input = r#"{"a": "${VAR1}", "b": "${VAR2}"}"#;
        let result = resolve_env_vars(input).unwrap();
        assert_eq!(result, r#"{"a": "value1", "b": "value2"}"#);
    }

    #[test]
    fn test_resolve_env_vars_no_vars() {
        let input = r#"{"token": "literal_value"}"#;
        let result = resolve_env_vars(input).unwrap();
        assert_eq!(result, r#"{"token": "literal_value"}"#);
    }

    #[test]
    fn test_resolve_env_vars_missing_var() {
        let input = r#"{"token": "${NONEXISTENT_VAR_12345}"}"#;
        let result = resolve_env_vars(input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, AppError::Config(_)));
    }

    // ==================== load_config Tests ====================

    #[test]
    #[serial]
    fn test_load_config_success() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.json");

        unsafe {
            std::env::set_var("TEST_ADMIN_TOKEN", "admin123");
            std::env::set_var("TEST_SEND_TOKEN", "send456");
            std::env::set_var("TEST_BACKEND_TOKEN", "backend789");
        }

        let config_content = r#"{
            "gateway": {
                "listen": "127.0.0.1:8080",
                "admin_token": "${TEST_ADMIN_TOKEN}",
                "default_target": {
                    "protocol": "pipelit",
                    "inbound_url": "http://localhost:9000/inbound",
                    "token": "${TEST_BACKEND_TOKEN}"
                }
            },
            "auth": {
                "send_token": "${TEST_SEND_TOKEN}"
            }
        }"#;

        std::fs::write(&config_path, config_content).unwrap();

        let config = load_config(&config_path).unwrap();
        assert_eq!(config.gateway.listen, "127.0.0.1:8080");
        assert_eq!(config.gateway.admin_token, "admin123");
        assert_eq!(config.auth.send_token, "send456");
        assert_eq!(config.gateway.default_target.token, "backend789");
    }

    #[test]
    fn test_load_config_file_not_found() {
        let result = load_config("/nonexistent/config.json");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, AppError::Config(_)));
    }

    #[test]
    fn test_load_config_invalid_json() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("invalid.json");
        std::fs::write(&config_path, "{ invalid json }").unwrap();

        let result = load_config(&config_path);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, AppError::Config(_)));
    }

    #[test]
    #[serial]
    fn test_load_config_with_defaults() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.json");

        unsafe {
            std::env::set_var("TEST_TOKEN_DEFAULT", "token123");
        }

        // Minimal config without optional fields
        let config_content = r#"{
            "gateway": {
                "listen": "127.0.0.1:8080",
                "admin_token": "${TEST_TOKEN_DEFAULT}",
                "default_target": {
                    "protocol": "pipelit",
                    "inbound_url": "http://localhost:9000/inbound",
                    "token": "${TEST_TOKEN_DEFAULT}"
                }
            },
            "auth": {
                "send_token": "${TEST_TOKEN_DEFAULT}"
            }
        }"#;

        std::fs::write(&config_path, config_content).unwrap();

        let config = load_config(&config_path).unwrap();
        // Check defaults are applied
        assert_eq!(config.gateway.adapters_dir, "./adapters");
        assert_eq!(config.gateway.adapter_port_range, (9000, 9100));
        assert!(config.gateway.file_cache.is_none());
        assert!(config.credentials.is_empty());
        assert!(config.health_checks.is_empty());
    }

    // ==================== Config Struct Tests ====================

    #[test]
    fn test_backend_protocol_serialize() {
        let pipelit = BackendProtocol::Pipelit;
        let json = serde_json::to_string(&pipelit).unwrap();
        assert_eq!(json, "\"pipelit\"");

        let opencode = BackendProtocol::Opencode;
        let json = serde_json::to_string(&opencode).unwrap();
        assert_eq!(json, "\"opencode\"");

        let external = BackendProtocol::External;
        let json = serde_json::to_string(&external).unwrap();
        assert_eq!(json, "\"external\"");
    }

    #[test]
    fn test_backend_protocol_deserialize() {
        let pipelit: BackendProtocol = serde_json::from_str("\"pipelit\"").unwrap();
        assert_eq!(pipelit, BackendProtocol::Pipelit);

        let opencode: BackendProtocol = serde_json::from_str("\"opencode\"").unwrap();
        assert_eq!(opencode, BackendProtocol::Opencode);

        let external: BackendProtocol = serde_json::from_str("\"external\"").unwrap();
        assert_eq!(external, BackendProtocol::External);
    }

    #[test]
    fn test_target_config_serialize() {
        let target = TargetConfig {
            protocol: BackendProtocol::Pipelit,
            inbound_url: Some("http://localhost:9000".to_string()),
            base_url: None,
            token: "test_token".to_string(),
            poll_interval_ms: None,
            adapter_dir: None,
            port: None,
        };

        let json = serde_json::to_string(&target).unwrap();
        assert!(json.contains("\"protocol\":\"pipelit\""));
        assert!(json.contains("\"token\":\"test_token\""));
    }

    #[test]
    fn test_target_config_opencode() {
        let json = r#"{
            "protocol": "opencode",
            "base_url": "http://localhost:8000",
            "token": "api_key",
            "poll_interval_ms": 1000
        }"#;

        let target: TargetConfig = serde_json::from_str(json).unwrap();
        assert_eq!(target.protocol, BackendProtocol::Opencode);
        assert_eq!(target.base_url, Some("http://localhost:8000".to_string()));
        assert_eq!(target.poll_interval_ms, Some(1000));
    }

    #[test]
    fn test_file_cache_config_serialize() {
        let cache = FileCacheConfig {
            directory: "/tmp/cache".to_string(),
            ttl_hours: 24,
            max_cache_size_mb: 100,
            cleanup_interval_minutes: 60,
            max_file_size_mb: 10,
            allowed_mime_types: vec!["image/*".to_string()],
            blocked_mime_types: vec![],
        };

        let json = serde_json::to_string(&cache).unwrap();
        assert!(json.contains("\"directory\":\"/tmp/cache\""));
        assert!(json.contains("\"ttl_hours\":24"));
    }

    #[test]
    fn test_health_check_config_serialize() {
        let check = HealthCheckConfig {
            url: "http://localhost:8080/health".to_string(),
            interval_seconds: 30,
            alert_after_failures: 3,
            notify_credentials: vec!["cred1".to_string()],
        };

        let json = serde_json::to_string(&check).unwrap();
        assert!(json.contains("\"interval_seconds\":30"));
    }

    #[test]
    fn test_credential_config_minimal() {
        let json = r#"{
            "adapter": "generic",
            "token": "token123",
            "active": true,
            "route": {"channel": "test"}
        }"#;

        let cred: CredentialConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cred.adapter, "generic");
        assert!(cred.active);
        assert!(!cred.emergency);
        assert!(cred.config.is_none());
        assert!(cred.target.is_none());
    }

    #[test]
    fn test_credential_config_full() {
        let json = r#"{
            "adapter": "telegram",
            "token": "bot_token",
            "active": true,
            "emergency": true,
            "config": {"webhook_url": "https://example.com"},
            "target": {
                "protocol": "opencode",
                "base_url": "http://localhost:8000",
                "token": "api_key"
            },
            "route": {"user_id": 123}
        }"#;

        let cred: CredentialConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cred.adapter, "telegram");
        assert!(cred.emergency);
        assert!(cred.config.is_some());
        assert!(cred.target.is_some());
    }

    #[test]
    fn test_auth_config_serialize() {
        let auth = AuthConfig {
            send_token: "secret_token".to_string(),
        };

        let json = serde_json::to_string(&auth).unwrap();
        assert!(json.contains("\"send_token\":\"secret_token\""));
    }

    #[test]
    fn test_gateway_config_serialize() {
        let gateway = GatewayConfig {
            listen: "0.0.0.0:8080".to_string(),
            admin_token: "admin123".to_string(),
            default_target: TargetConfig {
                protocol: BackendProtocol::Pipelit,
                inbound_url: Some("http://localhost:9000".to_string()),
                base_url: None,
                token: "backend_token".to_string(),
                poll_interval_ms: None,
                adapter_dir: None,
                port: None,
            },
            adapters_dir: "./adapters".to_string(),
            adapter_port_range: (9000, 9100),
            backends_dir: "./backends".to_string(),
            backend_port_range: (9200, 9300),
            file_cache: None,
        };

        let json = serde_json::to_string(&gateway).unwrap();
        assert!(json.contains("\"listen\":\"0.0.0.0:8080\""));
        assert!(json.contains("\"adapter_port_range\":[9000,9100]"));
        assert!(json.contains("\"backend_port_range\":[9200,9300]"));
    }

    #[test]
    fn test_config_full_roundtrip() {
        let config = Config {
            gateway: GatewayConfig {
                listen: "127.0.0.1:8080".to_string(),
                admin_token: "admin".to_string(),
                default_target: TargetConfig {
                    protocol: BackendProtocol::Pipelit,
                    inbound_url: Some("http://localhost:9000".to_string()),
                    base_url: None,
                    token: "token".to_string(),
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
                send_token: "send".to_string(),
            },
            health_checks: HashMap::new(),
            credentials: HashMap::new(),
        };

        let json = serde_json::to_string(&config).unwrap();
        let parsed: Config = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.gateway.listen, config.gateway.listen);
        assert_eq!(parsed.auth.send_token, config.auth.send_token);
    }

    // ==================== Default Function Tests ====================

    #[test]
    fn test_default_adapters_dir() {
        assert_eq!(default_adapters_dir(), "./adapters");
    }

    #[test]
    fn test_default_adapter_port_range() {
        assert_eq!(default_adapter_port_range(), (9000, 9100));
    }

    #[test]
    fn test_default_backends_dir() {
        assert_eq!(default_backends_dir(), "./backends");
    }

    #[test]
    fn test_default_backend_port_range() {
        assert_eq!(default_backend_port_range(), (9200, 9300));
    }
}
