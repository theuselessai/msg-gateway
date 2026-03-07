//! Backend adapter implementations for different backend protocols.
//!
//! Each backend adapter handles sending messages to a specific backend type
//! (Pipelit, OpenCode, etc.) and manages responses if needed.

use async_trait::async_trait;
use std::sync::Arc;

use crate::config::{BackendProtocol, CredentialConfig, TargetConfig};
use crate::message::InboundMessage;

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
    pub fn new(target: &TargetConfig) -> Result<Self, BackendError> {
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

/// OpenCode backend adapter - session-based with polling
/// TODO: Implement in Phase 3
pub struct OpencodeAdapter {
    #[allow(dead_code)]
    client: reqwest::Client,
    #[allow(dead_code)]
    base_url: String,
    #[allow(dead_code)]
    token: String,
    #[allow(dead_code)]
    poll_interval_ms: u64,
}

impl OpencodeAdapter {
    pub fn new(target: &TargetConfig) -> Result<Self, BackendError> {
        let base_url = target.base_url.clone().ok_or_else(|| {
            BackendError::InvalidConfig("OpenCode target requires base_url".to_string())
        })?;

        Ok(Self {
            client: reqwest::Client::new(),
            base_url,
            token: target.token.clone(),
            poll_interval_ms: target.poll_interval_ms.unwrap_or(500),
        })
    }
}

#[async_trait]
impl BackendAdapter for OpencodeAdapter {
    async fn send_message(&self, _message: &InboundMessage) -> Result<(), BackendError> {
        // TODO: Implement OpenCode session management and prompt_async
        // For now, return an error
        Err(BackendError::InvalidConfig(
            "OpenCode adapter not yet implemented".to_string(),
        ))
    }

    fn supports_files(&self) -> bool {
        false // OpenCode does not support file attachments
    }
}

/// Create a backend adapter from a target configuration
pub fn create_adapter(target: &TargetConfig) -> Result<Arc<dyn BackendAdapter>, BackendError> {
    match target.protocol {
        BackendProtocol::Pipelit => Ok(Arc::new(PipelitAdapter::new(target)?)),
        BackendProtocol::Opencode => Ok(Arc::new(OpencodeAdapter::new(target)?)),
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

    #[test]
    fn test_pipelit_adapter_requires_inbound_url() {
        let target = TargetConfig {
            protocol: BackendProtocol::Pipelit,
            inbound_url: None,
            base_url: None,
            token: "test".to_string(),
            poll_interval_ms: None,
        };

        let result = PipelitAdapter::new(&target);
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

        let adapter = PipelitAdapter::new(&target).unwrap();
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

        let result = OpencodeAdapter::new(&target);
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

        let adapter = OpencodeAdapter::new(&target).unwrap();
        assert!(!adapter.supports_files());
    }
}
