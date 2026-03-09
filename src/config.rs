use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub gateway: GatewayConfig,
    pub auth: AuthConfig,
    #[serde(default)]
    pub health_checks: HashMap<String, HealthCheckConfig>,
    #[serde(default)]
    pub credentials: HashMap<String, CredentialConfig>,
    #[serde(default)]
    pub backends: HashMap<String, BackendConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    pub listen: String,
    pub admin_token: String,
    #[serde(default)]
    pub default_backend: Option<String>,
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
    #[serde(default)]
    pub guardrails_dir: Option<String>,
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

fn default_true() -> bool {
    true
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

/// Guardrail evaluation type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
pub enum GuardrailType {
    /// CEL (Common Expression Language) evaluation
    #[default]
    Cel,
    /// LLM-based evaluation (placeholder for future)
    Llm,
}

/// Action to take when guardrail rule matches
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
pub enum GuardrailAction {
    /// Block the message
    #[default]
    Block,
    /// Log the violation
    Log,
}

/// Direction of message flow to apply guardrail
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
pub enum GuardrailDirection {
    /// Inbound messages only (adapter → gateway)
    #[default]
    Inbound,
    /// Outbound messages only (gateway → adapter)
    Outbound,
    /// Both inbound and outbound
    Both,
}

/// Behavior when guardrail evaluation errors
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
pub enum GuardrailOnError {
    /// Allow the message (fail-open)
    #[default]
    Allow,
    /// Block the message (fail-closed)
    Block,
}

/// Guardrail rule configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct GuardrailRule {
    /// Rule name
    pub name: String,
    /// Evaluation type
    #[serde(default)]
    pub r#type: GuardrailType,
    /// CEL expression to evaluate
    pub expression: String,
    /// Action when rule matches
    #[serde(default)]
    pub action: GuardrailAction,
    /// Message direction to apply rule
    #[serde(default)]
    pub direction: GuardrailDirection,
    /// Behavior on evaluation error
    #[serde(default)]
    pub on_error: GuardrailOnError,
    /// Message to return if blocked
    #[serde(default)]
    pub reject_message: Option<String>,
    /// Whether rule is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Backend configuration
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendConfig {
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
    /// Whether this backend is active (auto-spawned at startup for External protocol)
    #[serde(default = "default_true")]
    pub active: bool,
    /// Opaque config blob passed as BACKEND_CONFIG env var to external subprocess
    #[serde(default)]
    pub config: Option<serde_json::Value>,
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
    pub adapter: String,
    pub token: String,
    pub active: bool,
    #[serde(default)]
    pub emergency: bool,
    #[serde(default)]
    pub config: Option<serde_json::Value>,
    #[serde(default)]
    pub backend: Option<String>,
    pub route: serde_json::Value,
}

/// Load config from file, resolving environment variables
pub fn load_config<P: AsRef<Path>>(path: P) -> Result<Config, AppError> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)
        .map_err(|e| AppError::Config(format!("Failed to read config file: {}", e)))?;

    let resolved = resolve_env_vars(&content)?;

    let mut config: Config = serde_json::from_str(&resolved)
        .map_err(|e| AppError::Config(format!("Failed to parse config: {}", e)))?;

    // Validate backend name references
    if let Some(ref default_backend) = config.gateway.default_backend
        && !config.backends.contains_key(default_backend)
    {
        return Err(AppError::Config(format!(
            "default_backend '{}' not found in backends map",
            default_backend
        )));
    }

    for (cred_id, cred) in &config.credentials {
        if let Some(ref backend) = cred.backend
            && !config.backends.contains_key(backend)
        {
            return Err(AppError::Config(format!(
                "Credential '{}' references unknown backend '{}'",
                cred_id, backend
            )));
        }
    }

    let config_dir = path.parent().unwrap_or(Path::new("."));
    resolve_guardrails_dir(&mut config.gateway, config_dir);

    Ok(config)
}

fn resolve_guardrails_dir(gateway: &mut GatewayConfig, config_dir: &Path) {
    match &gateway.guardrails_dir {
        Some(dir) => {
            let p = Path::new(dir);
            if p.is_relative() {
                gateway.guardrails_dir = Some(config_dir.join(p).to_string_lossy().into_owned());
            }
        }
        None => {
            let auto = config_dir.join("guardrails");
            if auto.exists() {
                gateway.guardrails_dir = Some(auto.to_string_lossy().into_owned());
            }
        }
    }
}

/// Resolve config file path using XDG conventions.
///
/// Resolution order:
/// 1. `GATEWAY_CONFIG` env var — returned as-is (backward compat, no existence check)
/// 2. `$XDG_CONFIG_HOME/msg-gateway/config.json` — if file exists
/// 3. `$HOME/.config/msg-gateway/config.json` — if file exists
/// 4. `./config.json` — CWD fallback
pub fn resolve_config_path() -> PathBuf {
    if let Ok(path) = std::env::var("GATEWAY_CONFIG") {
        return PathBuf::from(path);
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        let p = PathBuf::from(xdg).join("msg-gateway").join("config.json");
        if p.exists() {
            return p;
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let p = PathBuf::from(home)
            .join(".config")
            .join("msg-gateway")
            .join("config.json");
        if p.exists() {
            return p;
        }
    }
    PathBuf::from("config.json")
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
                "default_backend": "pipelit"
            },
            "backends": {
                "pipelit": {
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
        assert_eq!(config.gateway.default_backend, Some("pipelit".to_string()));
        assert_eq!(config.backends["pipelit"].token, "backend789");
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
    fn test_load_config_invalid_default_backend() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.json");
        let content = r#"{
            "gateway": {"listen": "127.0.0.1:8080", "admin_token": "a", "default_backend": "nonexistent"},
            "auth": {"send_token": "s"}
        }"#;
        std::fs::write(&config_path, content).unwrap();
        let result = load_config(&config_path);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::Config(_)));
    }

    #[test]
    fn test_load_config_invalid_credential_backend() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.json");
        let content = r#"{
            "gateway": {"listen": "127.0.0.1:8080", "admin_token": "a"},
            "auth": {"send_token": "s"},
            "credentials": {
                "test_cred": {
                    "adapter": "generic",
                    "token": "token123",
                    "active": true,
                    "backend": "nonexistent",
                    "route": {"channel": "test"}
                }
            }
        }"#;
        std::fs::write(&config_path, content).unwrap();
        let result = load_config(&config_path);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::Config(_)));
    }

    #[test]
    #[serial]
    fn test_load_config_with_defaults() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.json");

        unsafe {
            std::env::set_var("TEST_TOKEN_DEFAULT", "token123");
        }

        let config_content = r#"{
            "gateway": {
                "listen": "127.0.0.1:8080",
                "admin_token": "${TEST_TOKEN_DEFAULT}"
            },
            "auth": {
                "send_token": "${TEST_TOKEN_DEFAULT}"
            }
        }"#;

        std::fs::write(&config_path, config_content).unwrap();

        let config = load_config(&config_path).unwrap();
        assert_eq!(config.gateway.adapters_dir, "./adapters");
        assert_eq!(config.gateway.adapter_port_range, (9000, 9100));
        assert!(config.gateway.file_cache.is_none());
        assert!(config.gateway.default_backend.is_none());
        assert!(config.credentials.is_empty());
        assert!(config.health_checks.is_empty());
        assert!(config.backends.is_empty());
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
    fn test_backend_config_serialize() {
        let backend = BackendConfig {
            protocol: BackendProtocol::Pipelit,
            inbound_url: Some("http://localhost:9000".to_string()),
            base_url: None,
            token: "test_token".to_string(),
            poll_interval_ms: None,
            adapter_dir: None,
            port: None,
            active: true,
            config: None,
        };

        let json = serde_json::to_string(&backend).unwrap();
        assert!(json.contains("\"protocol\":\"pipelit\""));
        assert!(json.contains("\"token\":\"test_token\""));
        assert!(json.contains("\"active\":true"));
    }

    #[test]
    fn test_backend_config_opencode() {
        let json = r#"{
            "protocol": "opencode",
            "base_url": "http://localhost:8000",
            "token": "api_key",
            "poll_interval_ms": 1000
        }"#;

        let backend: BackendConfig = serde_json::from_str(json).unwrap();
        assert_eq!(backend.protocol, BackendProtocol::Opencode);
        assert_eq!(backend.base_url, Some("http://localhost:8000".to_string()));
        assert_eq!(backend.poll_interval_ms, Some(1000));
        assert!(backend.active);
        assert!(backend.config.is_none());
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
        assert!(cred.backend.is_none());
    }

    #[test]
    fn test_credential_config_full() {
        let json = r#"{
            "adapter": "telegram",
            "token": "bot_token",
            "active": true,
            "emergency": true,
            "config": {"webhook_url": "https://example.com"},
            "backend": "opencode",
            "route": {"user_id": 123}
        }"#;

        let cred: CredentialConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cred.adapter, "telegram");
        assert!(cred.emergency);
        assert!(cred.config.is_some());
        assert_eq!(cred.backend, Some("opencode".to_string()));
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
            default_backend: Some("opencode".to_string()),
            adapters_dir: "./adapters".to_string(),
            adapter_port_range: (9000, 9100),
            backends_dir: "./backends".to_string(),
            backend_port_range: (9200, 9300),
            file_cache: None,
            guardrails_dir: None,
        };

        let json = serde_json::to_string(&gateway).unwrap();
        assert!(json.contains("\"listen\":\"0.0.0.0:8080\""));
        assert!(json.contains("\"default_backend\":\"opencode\""));
        assert!(json.contains("\"adapter_port_range\":[9000,9100]"));
        assert!(json.contains("\"backend_port_range\":[9200,9300]"));
    }

    #[test]
    fn test_config_full_roundtrip() {
        let mut backends = HashMap::new();
        backends.insert(
            "pipelit".to_string(),
            BackendConfig {
                protocol: BackendProtocol::Pipelit,
                inbound_url: Some("http://localhost:9000".to_string()),
                base_url: None,
                token: "token".to_string(),
                poll_interval_ms: None,
                adapter_dir: None,
                port: None,
                active: true,
                config: None,
            },
        );

        let config = Config {
            gateway: GatewayConfig {
                listen: "127.0.0.1:8080".to_string(),
                admin_token: "admin".to_string(),
                default_backend: Some("pipelit".to_string()),
                adapters_dir: "./adapters".to_string(),
                adapter_port_range: (9000, 9100),
                backends_dir: "./backends".to_string(),
                backend_port_range: (9200, 9300),
                file_cache: None,
                guardrails_dir: None,
            },
            auth: AuthConfig {
                send_token: "send".to_string(),
            },
            health_checks: HashMap::new(),
            credentials: HashMap::new(),
            backends,
        };

        let json = serde_json::to_string(&config).unwrap();
        let parsed: Config = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.gateway.listen, config.gateway.listen);
        assert_eq!(parsed.auth.send_token, config.auth.send_token);
        assert_eq!(
            parsed.gateway.default_backend,
            config.gateway.default_backend
        );
        assert_eq!(parsed.backends.len(), 1);
        assert!(parsed.backends.contains_key("pipelit"));
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

    // ==================== New Named-Backends Tests ====================

    #[test]
    fn test_backends_deserialization() {
        let json = r#"{
            "gateway": {
                "listen": "127.0.0.1:8080",
                "admin_token": "admin",
                "default_backend": "opencode"
            },
            "backends": {
                "opencode": {
                    "protocol": "external",
                    "adapter_dir": "./backends/opencode",
                    "active": true,
                    "token": "",
                    "config": {"base_url": "http://127.0.0.1:4096"}
                },
                "pipelit": {
                    "protocol": "pipelit",
                    "inbound_url": "http://localhost:8000/api/v1/inbound",
                    "token": "pipelit-token",
                    "active": true
                }
            },
            "auth": {
                "send_token": "send-token"
            }
        }"#;

        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.backends.len(), 2);

        let opencode = &config.backends["opencode"];
        assert_eq!(opencode.protocol, BackendProtocol::External);
        assert_eq!(
            opencode.adapter_dir,
            Some("./backends/opencode".to_string())
        );
        assert!(opencode.active);
        assert_eq!(opencode.token, "");
        assert!(opencode.config.is_some());

        let pipelit = &config.backends["pipelit"];
        assert_eq!(pipelit.protocol, BackendProtocol::Pipelit);
        assert_eq!(
            pipelit.inbound_url,
            Some("http://localhost:8000/api/v1/inbound".to_string())
        );
        assert!(pipelit.active);
        assert_eq!(pipelit.token, "pipelit-token");
        assert!(pipelit.config.is_none());
    }

    #[test]
    fn test_credential_backend_field() {
        let json = r#"{
            "adapter": "telegram",
            "token": "bot_token",
            "active": true,
            "backend": "opencode",
            "route": {"channel": "telegram"}
        }"#;

        let cred: CredentialConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cred.backend, Some("opencode".to_string()));
        assert_eq!(cred.adapter, "telegram");
    }

    #[test]
    fn test_default_backend_field_serde() {
        let json = r#"{
            "gateway": {
                "listen": "127.0.0.1:8080",
                "admin_token": "admin",
                "default_backend": "opencode"
            },
            "auth": {
                "send_token": "token"
            }
        }"#;

        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.gateway.default_backend, Some("opencode".to_string()));
        assert!(config.backends.is_empty());
    }

    // ==================== GuardrailRule Tests ====================

    #[test]
    fn test_guardrail_rule_minimal_json() {
        let json = r#"{"name":"test","expression":"true"}"#;
        let rule: GuardrailRule = serde_json::from_str(json).unwrap();

        assert_eq!(rule.name, "test");
        assert_eq!(rule.expression, "true");
        assert_eq!(rule.r#type, GuardrailType::Cel);
        assert_eq!(rule.action, GuardrailAction::Block);
        assert_eq!(rule.direction, GuardrailDirection::Inbound);
        assert_eq!(rule.on_error, GuardrailOnError::Allow);
        assert_eq!(rule.reject_message, None);
        assert!(rule.enabled);
    }

    #[test]
    fn test_guardrail_rule_full_json() {
        let json = r#"{
            "name":"test_rule",
            "type":"cel",
            "expression":"message.text.size() > 100",
            "action":"log",
            "direction":"both",
            "on_error":"block",
            "reject_message":"Message too long",
            "enabled":false
        }"#;
        let rule: GuardrailRule = serde_json::from_str(json).unwrap();

        assert_eq!(rule.name, "test_rule");
        assert_eq!(rule.r#type, GuardrailType::Cel);
        assert_eq!(rule.expression, "message.text.size() > 100");
        assert_eq!(rule.action, GuardrailAction::Log);
        assert_eq!(rule.direction, GuardrailDirection::Both);
        assert_eq!(rule.on_error, GuardrailOnError::Block);
        assert_eq!(rule.reject_message, Some("Message too long".to_string()));
        assert!(!rule.enabled);
    }

    #[test]
    fn test_guardrail_rule_enabled_default() {
        let json = r#"{"name":"test","expression":"true"}"#;
        let rule: GuardrailRule = serde_json::from_str(json).unwrap();
        assert!(rule.enabled);
    }

    #[test]
    fn test_guardrail_rule_enabled_false() {
        let json = r#"{"name":"test","expression":"true","enabled":false}"#;
        let rule: GuardrailRule = serde_json::from_str(json).unwrap();
        assert!(!rule.enabled);
    }

    #[test]
    fn test_guardrail_rule_roundtrip() {
        let rule = GuardrailRule {
            name: "test".to_string(),
            r#type: GuardrailType::Cel,
            expression: "true".to_string(),
            action: GuardrailAction::Block,
            direction: GuardrailDirection::Inbound,
            on_error: GuardrailOnError::Allow,
            reject_message: Some("rejected".to_string()),
            enabled: true,
        };

        let json = serde_json::to_string(&rule).unwrap();
        let parsed: GuardrailRule = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.name, rule.name);
        assert_eq!(parsed.r#type, rule.r#type);
        assert_eq!(parsed.expression, rule.expression);
        assert_eq!(parsed.action, rule.action);
        assert_eq!(parsed.direction, rule.direction);
        assert_eq!(parsed.on_error, rule.on_error);
        assert_eq!(parsed.reject_message, rule.reject_message);
        assert_eq!(parsed.enabled, rule.enabled);
    }

    #[test]
    fn test_guardrail_type_default() {
        let json = r#"{"name":"test","expression":"true"}"#;
        let rule: GuardrailRule = serde_json::from_str(json).unwrap();
        assert_eq!(rule.r#type, GuardrailType::Cel);
    }

    #[test]
    fn test_guardrail_action_default() {
        let json = r#"{"name":"test","expression":"true"}"#;
        let rule: GuardrailRule = serde_json::from_str(json).unwrap();
        assert_eq!(rule.action, GuardrailAction::Block);
    }

    #[test]
    fn test_guardrail_direction_default() {
        let json = r#"{"name":"test","expression":"true"}"#;
        let rule: GuardrailRule = serde_json::from_str(json).unwrap();
        assert_eq!(rule.direction, GuardrailDirection::Inbound);
    }

    #[test]
    fn test_guardrail_on_error_default() {
        let json = r#"{"name":"test","expression":"true"}"#;
        let rule: GuardrailRule = serde_json::from_str(json).unwrap();
        assert_eq!(rule.on_error, GuardrailOnError::Allow);
    }

    #[test]
    fn test_guardrail_invalid_action_error() {
        let json = r#"{"name":"test","expression":"true","action":"invalid"}"#;
        let result: Result<GuardrailRule, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_guardrail_invalid_type_error() {
        let json = r#"{"name":"test","expression":"true","type":"invalid"}"#;
        let result: Result<GuardrailRule, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_guardrail_invalid_direction_error() {
        let json = r#"{"name":"test","expression":"true","direction":"invalid"}"#;
        let result: Result<GuardrailRule, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_guardrail_invalid_on_error_error() {
        let json = r#"{"name":"test","expression":"true","on_error":"invalid"}"#;
        let result: Result<GuardrailRule, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    // ==================== resolve_config_path Tests ====================

    #[test]
    #[serial]
    fn test_resolve_config_path_env_override() {
        unsafe {
            std::env::set_var("GATEWAY_CONFIG", "/tmp/custom.json");
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::remove_var("HOME");
        }
        let result = resolve_config_path();
        unsafe {
            std::env::remove_var("GATEWAY_CONFIG");
        }
        assert_eq!(result, PathBuf::from("/tmp/custom.json"));
    }

    #[test]
    #[serial]
    fn test_resolve_config_path_xdg_config_home() {
        let temp_dir = TempDir::new().unwrap();
        let xdg_config = temp_dir.path().join("msg-gateway");
        std::fs::create_dir_all(&xdg_config).unwrap();
        let config_file = xdg_config.join("config.json");
        std::fs::write(&config_file, "{}").unwrap();

        unsafe {
            std::env::remove_var("GATEWAY_CONFIG");
            std::env::set_var("XDG_CONFIG_HOME", temp_dir.path());
            std::env::remove_var("HOME");
        }
        let result = resolve_config_path();
        unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
        }
        assert_eq!(result, config_file);
    }

    #[test]
    #[serial]
    fn test_resolve_config_path_home_config() {
        let temp_dir = TempDir::new().unwrap();
        let home_config = temp_dir.path().join(".config").join("msg-gateway");
        std::fs::create_dir_all(&home_config).unwrap();
        let config_file = home_config.join("config.json");
        std::fs::write(&config_file, "{}").unwrap();

        unsafe {
            std::env::remove_var("GATEWAY_CONFIG");
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::set_var("HOME", temp_dir.path());
        }
        let result = resolve_config_path();
        unsafe {
            std::env::remove_var("HOME");
        }
        assert_eq!(result, config_file);
    }

    #[test]
    #[serial]
    fn test_resolve_config_path_cwd_fallback() {
        unsafe {
            std::env::remove_var("GATEWAY_CONFIG");
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::remove_var("HOME");
        }
        let result = resolve_config_path();
        assert_eq!(result, PathBuf::from("config.json"));
    }

    #[test]
    #[serial]
    fn test_resolve_config_path_xdg_takes_precedence_over_home() {
        let temp_dir = TempDir::new().unwrap();

        let xdg_config = temp_dir.path().join("xdg").join("msg-gateway");
        std::fs::create_dir_all(&xdg_config).unwrap();
        let xdg_file = xdg_config.join("config.json");
        std::fs::write(&xdg_file, "{}").unwrap();

        let home_config = temp_dir
            .path()
            .join("home")
            .join(".config")
            .join("msg-gateway");
        std::fs::create_dir_all(&home_config).unwrap();
        let home_file = home_config.join("config.json");
        std::fs::write(&home_file, "{}").unwrap();

        unsafe {
            std::env::remove_var("GATEWAY_CONFIG");
            std::env::set_var("XDG_CONFIG_HOME", temp_dir.path().join("xdg"));
            std::env::set_var("HOME", temp_dir.path().join("home"));
        }
        let result = resolve_config_path();
        unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::remove_var("HOME");
        }
        assert_eq!(result, xdg_file);
    }

    // ==================== guardrails_dir Tests ====================

    fn make_minimal_gateway(guardrails_dir: Option<String>) -> GatewayConfig {
        GatewayConfig {
            listen: "127.0.0.1:8080".to_string(),
            admin_token: "token".to_string(),
            default_backend: None,
            adapters_dir: "./adapters".to_string(),
            adapter_port_range: (9000, 9100),
            backends_dir: "./backends".to_string(),
            backend_port_range: (9200, 9300),
            file_cache: None,
            guardrails_dir,
        }
    }

    #[test]
    fn test_guardrails_dir_auto_discovery() {
        let temp_dir = TempDir::new().unwrap();
        let guardrails_path = temp_dir.path().join("guardrails");
        std::fs::create_dir_all(&guardrails_path).unwrap();

        let mut gateway = make_minimal_gateway(None);
        resolve_guardrails_dir(&mut gateway, temp_dir.path());

        assert_eq!(
            gateway.guardrails_dir,
            Some(guardrails_path.to_string_lossy().into_owned())
        );
    }

    #[test]
    fn test_guardrails_dir_none_no_dir() {
        let temp_dir = TempDir::new().unwrap();
        let mut gateway = make_minimal_gateway(None);
        resolve_guardrails_dir(&mut gateway, temp_dir.path());
        assert_eq!(gateway.guardrails_dir, None);
    }

    #[test]
    fn test_guardrails_dir_relative_resolved() {
        let temp_dir = TempDir::new().unwrap();
        let mut gateway = make_minimal_gateway(Some("./my_rules".to_string()));
        resolve_guardrails_dir(&mut gateway, temp_dir.path());
        let result = gateway.guardrails_dir.unwrap();
        assert!(
            result.contains("my_rules"),
            "Expected path to contain 'my_rules', got: {}",
            result
        );
        assert!(
            result.starts_with(temp_dir.path().to_str().unwrap()),
            "Expected path to start with temp dir"
        );
    }

    #[test]
    fn test_guardrails_dir_absolute_unchanged() {
        let temp_dir = TempDir::new().unwrap();
        let abs_path = "/absolute/path/to/rules".to_string();
        let mut gateway = make_minimal_gateway(Some(abs_path.clone()));
        resolve_guardrails_dir(&mut gateway, temp_dir.path());
        assert_eq!(gateway.guardrails_dir, Some(abs_path));
    }

    #[test]
    fn test_guardrails_dir_serde_absent() {
        let json = r#"{
            "listen": "127.0.0.1:8080",
            "admin_token": "tok"
        }"#;
        let gw: GatewayConfig = serde_json::from_str(json).unwrap();
        assert_eq!(gw.guardrails_dir, None);
    }

    #[test]
    fn test_guardrails_dir_field_in_gateway_config() {
        let json = r#"{
            "listen": "127.0.0.1:8080",
            "admin_token": "tok",
            "guardrails_dir": "/my/rules"
        }"#;
        let gw: GatewayConfig = serde_json::from_str(json).unwrap();
        assert_eq!(gw.guardrails_dir, Some("/my/rules".to_string()));
    }

    #[test]
    fn test_guardrails_dir_absent_defaults_none() {
        let json = r#"{
            "listen": "127.0.0.1:8080",
            "admin_token": "tok"
        }"#;
        let gw: GatewayConfig = serde_json::from_str(json).unwrap();
        assert_eq!(gw.guardrails_dir, None);
    }
}
