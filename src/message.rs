use serde::{Deserialize, Serialize};

/// Normalized inbound message envelope (Gateway → Pipelit)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    pub route: serde_json::Value,
    pub credential_id: String,
    pub source: MessageSource,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSource {
    pub protocol: String,
    pub chat_id: String,
    pub message_id: String,
    pub from: UserInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
    /// Gateway-hosted download URL for successful files, or "error: ..." for failed downloads
    pub download_url: String,
}

/// Outbound message (Pipelit → Gateway → Client)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub credential_id: String,
    pub chat_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
    pub text: String,
}

/// Response after sending a message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendResponse {
    pub status: String,
    pub protocol_message_id: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// WebSocket message pushed to clients
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsOutboundMessage {
    pub text: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub message_id: String,
}
