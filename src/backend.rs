//! Backend adapter implementations for different backend protocols.
//!
//! Each backend adapter handles sending messages to a specific backend type
//! (Pipelit, OpenCode, etc.) and manages responses if needed.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

use crate::config::{BackendProtocol, CredentialConfig, TargetConfig};
use crate::message::InboundMessage;

/// Gateway context passed to backend adapters that need to call back into the gateway
pub struct GatewayContext {
    pub gateway_url: String,
    pub send_token: String,
}

/// Shared session map for OpenCode adapters (persists across per-request adapter creation).
/// Uses a Mutex (not RwLock) so that session creation is serialized per-process,
/// preventing duplicate session creation under concurrent requests for the same chat_id.
static OPENCODE_SESSIONS: OnceLock<Arc<Mutex<HashMap<String, String>>>> = OnceLock::new();

fn get_opencode_sessions() -> Arc<Mutex<HashMap<String, String>>> {
    OPENCODE_SESSIONS
        .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
        .clone()
}

/// Error type for backend operations
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Backend returned error: {status} - {message}")]
    #[allow(clippy::enum_variant_names)]
    BackendResponse { status: u16, message: String },

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("Timeout waiting for response")]
    #[allow(dead_code)]
    Timeout,
}

/// Trait for backend adapters
#[async_trait]
pub trait BackendAdapter: Send + Sync {
    /// Send a normalized message to the backend
    async fn send_message(&self, message: &InboundMessage) -> Result<(), BackendError>;

    /// Whether this backend supports file attachments
    #[allow(dead_code)]
    fn supports_files(&self) -> bool;
}

/// Pipelit backend adapter - fire-and-forget webhook
pub struct PipelitAdapter {
    client: reqwest::Client,
    inbound_url: String,
    token: String,
}

impl PipelitAdapter {
    pub fn new(
        target: &TargetConfig,
        _gateway_ctx: Option<&GatewayContext>,
        _credential_config: Option<&serde_json::Value>,
    ) -> Result<Self, BackendError> {
        let inbound_url = target.inbound_url.clone().ok_or_else(|| {
            BackendError::InvalidConfig("Pipelit target requires inbound_url".to_string())
        })?;

        Ok(Self {
            client: reqwest::Client::new(),
            inbound_url,
            token: target.token.clone(),
        })
    }
}

#[async_trait]
impl BackendAdapter for PipelitAdapter {
    async fn send_message(&self, message: &InboundMessage) -> Result<(), BackendError> {
        let response = self
            .client
            .post(&self.inbound_url)
            .header("Authorization", format!("Bearer {}", self.token))
            .json(message)
            .send()
            .await?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status().as_u16();
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            Err(BackendError::BackendResponse { status, message })
        }
    }

    fn supports_files(&self) -> bool {
        true
    }
}

pub struct OpencodeAdapter {
    base_url: String,
    token: String,
    gateway_url: String,
    send_token: String,
    credential_config: Option<serde_json::Value>,
    sessions: Arc<Mutex<HashMap<String, String>>>,
}

impl OpencodeAdapter {
    pub fn new(
        target: &TargetConfig,
        gateway_ctx: Option<&GatewayContext>,
        credential_config: Option<&serde_json::Value>,
    ) -> Result<Self, BackendError> {
        let base_url = target.base_url.clone().ok_or_else(|| {
            BackendError::InvalidConfig("OpenCode target requires base_url".to_string())
        })?;

        Ok(Self {
            base_url,
            token: target.token.clone(),
            gateway_url: gateway_ctx
                .map(|ctx| ctx.gateway_url.clone())
                .unwrap_or_default(),
            send_token: gateway_ctx
                .map(|ctx| ctx.send_token.clone())
                .unwrap_or_default(),
            credential_config: credential_config.cloned(),
            sessions: get_opencode_sessions(),
        })
    }
}

#[async_trait]
impl BackendAdapter for OpencodeAdapter {
    async fn send_message(&self, message: &InboundMessage) -> Result<(), BackendError> {
        // Parse auth credentials (split at first colon)
        let colon_pos = self.token.find(':').ok_or_else(|| {
            BackendError::InvalidConfig(
                "OpenCode token must be in 'username:password' format".to_string(),
            )
        })?;
        let username = &self.token[..colon_pos];
        let password = &self.token[colon_pos + 1..];

        let model = self
            .credential_config
            .as_ref()
            .and_then(|c| c.get("model"))
            .ok_or_else(|| {
                BackendError::InvalidConfig("Missing 'model' in credential config".to_string())
            })?;

        // Build client with 120s timeout for LLM calls
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;

        let chat_id = &message.source.chat_id;
        let session_key = format!("{}:{}", message.credential_id, chat_id);

        let session_id = {
            let mut sessions = self.sessions.lock().await;
            if let Some(id) = sessions.get(&session_key) {
                id.clone()
            } else {
                tracing::info!(
                    credential_id = %message.credential_id,
                    chat_id = %chat_id,
                    "Creating new OpenCode session"
                );

                let resp = client
                    .post(format!("{}/session", self.base_url))
                    .basic_auth(username, Some(password))
                    .send()
                    .await?;

                if !resp.status().is_success() {
                    let status = resp.status().as_u16();
                    let body = resp
                        .text()
                        .await
                        .unwrap_or_else(|_| "Unknown error".to_string());
                    return Err(BackendError::BackendResponse {
                        status,
                        message: body,
                    });
                }

                let body: serde_json::Value = resp.json().await?;
                let new_session_id = body
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        BackendError::InvalidConfig(
                            "OpenCode session response missing 'id' field".to_string(),
                        )
                    })?
                    .to_string();

                sessions.insert(session_key.clone(), new_session_id.clone());
                new_session_id
            }
        };

        tracing::info!(
            credential_id = %message.credential_id,
            chat_id = %chat_id,
            session_id = %session_id,
            "Sending message to OpenCode"
        );

        let msg_body = serde_json::json!({
            "model": model,
            "parts": [{"type": "text", "text": message.text}]
        });

        let resp = client
            .post(format!("{}/session/{}/message", self.base_url, session_id))
            .basic_auth(username, Some(password))
            .json(&msg_body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(BackendError::BackendResponse {
                status,
                message: body,
            });
        }

        // Extract AI response: join all text parts with "\n\n"
        let resp_body: serde_json::Value = resp.json().await?;
        let ai_response = resp_body
            .get("parts")
            .and_then(|v| v.as_array())
            .map(|parts| {
                parts
                    .iter()
                    .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
                    .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            })
            .unwrap_or_default();

        // Self-relay: send AI response back through gateway to the user
        tracing::info!(
            credential_id = %message.credential_id,
            chat_id = %chat_id,
            "Relaying OpenCode response to gateway"
        );

        let relay_body = serde_json::json!({
            "credential_id": message.credential_id,
            "chat_id": chat_id,
            "text": ai_response,
        });

        match client
            .post(format!("{}/api/v1/send", self.gateway_url))
            .header("Authorization", format!("Bearer {}", self.send_token))
            .json(&relay_body)
            .send()
            .await
        {
            Ok(resp) if !resp.status().is_success() => {
                let status = resp.status();
                tracing::error!(
                    credential_id = %message.credential_id,
                    chat_id = %chat_id,
                    error = %format!("HTTP {}", status),
                    "Failed to relay OpenCode response"
                );
            }
            Err(e) => {
                tracing::error!(
                    credential_id = %message.credential_id,
                    chat_id = %chat_id,
                    error = %e,
                    "Failed to relay OpenCode response"
                );
            }
            Ok(_) => {}
        }

        Ok(())
    }

    fn supports_files(&self) -> bool {
        false // OpenCode does not support file attachments
    }
}

pub fn create_adapter(
    target: &TargetConfig,
    gateway_ctx: Option<&GatewayContext>,
    credential_config: Option<&serde_json::Value>,
) -> Result<Arc<dyn BackendAdapter>, BackendError> {
    match target.protocol {
        BackendProtocol::Pipelit => Ok(Arc::new(PipelitAdapter::new(
            target,
            gateway_ctx,
            credential_config,
        )?)),
        BackendProtocol::Opencode => Ok(Arc::new(OpencodeAdapter::new(
            target,
            gateway_ctx,
            credential_config,
        )?)),
    }
}

/// Resolve the target configuration for a credential
/// Returns the credential's target if set, otherwise falls back to default_target
pub fn resolve_target<'a>(
    credential: &'a CredentialConfig,
    default_target: &'a TargetConfig,
) -> &'a TargetConfig {
    credential.target.as_ref().unwrap_or(default_target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BackendProtocol;
    use crate::message::{InboundMessage, MessageSource, UserInfo};

    fn make_opencode_target(token: &str) -> TargetConfig {
        TargetConfig {
            protocol: BackendProtocol::Opencode,
            inbound_url: None,
            base_url: Some("http://localhost:4096".to_string()),
            token: token.to_string(),
            poll_interval_ms: None,
        }
    }

    fn make_dummy_message() -> InboundMessage {
        InboundMessage {
            route: serde_json::json!({}),
            credential_id: "test_cred".to_string(),
            source: MessageSource {
                protocol: "test".to_string(),
                chat_id: "chat_123".to_string(),
                message_id: "msg_1".to_string(),
                reply_to_message_id: None,
                from: UserInfo {
                    id: "user_1".to_string(),
                    username: None,
                    display_name: None,
                },
            },
            text: "Hello".to_string(),
            attachments: vec![],
            timestamp: chrono::Utc::now(),
            extra_data: None,
        }
    }

    #[test]
    fn test_pipelit_adapter_requires_inbound_url() {
        let target = TargetConfig {
            protocol: BackendProtocol::Pipelit,
            inbound_url: None,
            base_url: None,
            token: "test".to_string(),
            poll_interval_ms: None,
        };

        let result = PipelitAdapter::new(&target, None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_pipelit_adapter_creation() {
        let target = TargetConfig {
            protocol: BackendProtocol::Pipelit,
            inbound_url: Some("http://localhost:8000/inbound".to_string()),
            base_url: None,
            token: "test".to_string(),
            poll_interval_ms: None,
        };

        let adapter = PipelitAdapter::new(&target, None, None).unwrap();
        assert!(adapter.supports_files());
    }

    #[test]
    fn test_opencode_adapter_requires_base_url() {
        let target = TargetConfig {
            protocol: BackendProtocol::Opencode,
            inbound_url: None,
            base_url: None,
            token: "test".to_string(),
            poll_interval_ms: None,
        };

        let result = OpencodeAdapter::new(&target, None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_opencode_adapter_creation() {
        let target = TargetConfig {
            protocol: BackendProtocol::Opencode,
            inbound_url: None,
            base_url: Some("http://localhost:4096".to_string()),
            token: "test".to_string(),
            poll_interval_ms: Some(1000),
        };

        let adapter = OpencodeAdapter::new(&target, None, None).unwrap();
        assert!(!adapter.supports_files());
    }

    #[test]
    fn test_opencode_adapter_valid_token_parsing() {
        let target = make_opencode_target("myuser:mypass");
        let result = OpencodeAdapter::new(&target, None, None);
        assert!(result.is_ok(), "Adapter with valid token should succeed");
    }

    #[tokio::test]
    async fn test_opencode_adapter_token_no_colon() {
        let target = make_opencode_target("nodelimiter");
        let adapter = OpencodeAdapter::new(&target, None, None).unwrap();
        let msg = make_dummy_message();
        let result = adapter.send_message(&msg).await;
        assert!(result.is_err());
        let err_str = result.unwrap_err().to_string();
        assert!(
            err_str.contains("username:password"),
            "Error should mention 'username:password', got: {err_str}"
        );
    }

    #[test]
    fn test_opencode_adapter_token_colon_in_password() {
        let target = make_opencode_target("user:pass:with:colons");
        let result = OpencodeAdapter::new(&target, None, None);
        assert!(
            result.is_ok(),
            "Token with colon in password should be accepted"
        );
    }

    #[tokio::test]
    async fn test_opencode_adapter_missing_credential_config() {
        let target = make_opencode_target("user:pass");
        let adapter = OpencodeAdapter::new(&target, None, None).unwrap();
        let msg = make_dummy_message();
        let result = adapter.send_message(&msg).await;
        assert!(result.is_err());
        let err_str = result.unwrap_err().to_string();
        assert!(
            err_str.contains("model"),
            "Error should mention 'model', got: {err_str}"
        );
    }

    #[tokio::test]
    async fn test_opencode_adapter_missing_model_in_config() {
        let target = make_opencode_target("user:pass");
        let config = serde_json::json!({});
        let adapter = OpencodeAdapter::new(&target, None, Some(&config)).unwrap();
        let msg = make_dummy_message();
        let result = adapter.send_message(&msg).await;
        assert!(result.is_err());
        let err_str = result.unwrap_err().to_string();
        assert!(
            err_str.contains("model"),
            "Error should mention 'model', got: {err_str}"
        );
    }

    #[test]
    fn test_opencode_adapter_valid_model_config() {
        let target = make_opencode_target("user:pass");
        let config = serde_json::json!({
            "model": {
                "providerID": "test",
                "modelID": "test-model"
            }
        });
        let result = OpencodeAdapter::new(&target, None, Some(&config));
        assert!(
            result.is_ok(),
            "Adapter with valid model config should succeed"
        );
    }

    #[test]
    fn test_opencode_adapter_creation_with_gateway_ctx() {
        let target = make_opencode_target("user:pass");
        let config = serde_json::json!({
            "model": {
                "providerID": "test",
                "modelID": "test-model"
            }
        });
        let gateway_ctx = GatewayContext {
            gateway_url: "http://localhost:8080".to_string(),
            send_token: "test_token".to_string(),
        };
        let result = OpencodeAdapter::new(&target, Some(&gateway_ctx), Some(&config));
        assert!(
            result.is_ok(),
            "Adapter with full valid config should succeed"
        );
    }

    #[test]
    fn test_opencode_adapter_supports_files_false() {
        let target = make_opencode_target("user:pass");
        let adapter = OpencodeAdapter::new(&target, None, None).unwrap();
        assert!(
            !adapter.supports_files(),
            "OpencodeAdapter should not support files"
        );
    }
}
