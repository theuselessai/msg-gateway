//! HTTP + WebSocket client for the msg-gateway API.

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use reqwest::header;
use serde::{Deserialize, Serialize};
use tokio_tungstenite::tungstenite;

// ---------------------------------------------------------------------------
// Response / request types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ChatRequest {
    pub chat_id: String,
    pub text: String,
    pub from: ChatUser,
}

#[derive(Debug, Serialize)]
pub struct ChatUser {
    pub id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ChatResponse {
    pub message_id: String,
    pub timestamp: String,
}

/// A message received on the WebSocket (outbound from gateway).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WsMessage {
    pub text: String,
    pub timestamp: String,
    pub message_id: String,
    #[serde(default)]
    pub file_urls: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct HealthResponse {
    pub status: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CredentialInfo {
    pub id: String,
    pub adapter: String,
    pub active: bool,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub route: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct CredentialListResponse {
    pub credentials: Vec<CredentialInfo>,
}

#[derive(Debug, Serialize)]
pub struct CreateCredentialRequest {
    pub id: String,
    pub adapter: String,
    pub token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
    pub active: bool,
}

// ---------------------------------------------------------------------------
// Gateway HTTP Client
// ---------------------------------------------------------------------------

pub struct GatewayClient {
    http: reqwest::Client,
    pub base_url: String,
}

impl GatewayClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    // -- Chat / Send (uses credential token) --------------------------------

    pub async fn send_chat_message(
        &self,
        credential_id: &str,
        chat_id: &str,
        text: &str,
        user_id: &str,
        token: &str,
    ) -> Result<ChatResponse> {
        let url = format!("{}/api/v1/chat/{}", self.base_url, credential_id);
        let body = ChatRequest {
            chat_id: chat_id.to_string(),
            text: text.to_string(),
            from: ChatUser {
                id: user_id.to_string(),
            },
        };

        let resp = self
            .http
            .post(&url)
            .header(header::AUTHORIZATION, format!("Bearer {}", token))
            .json(&body)
            .send()
            .await
            .context("Failed to connect to gateway")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Gateway returned {} — {}", status, body);
        }

        resp.json::<ChatResponse>()
            .await
            .context("Failed to parse chat response")
    }

    // -- WebSocket (uses credential token) ----------------------------------

    /// Connect to the WebSocket endpoint and return the stream.
    /// The returned stream yields `WsMessage` items.
    pub async fn connect_ws(
        &self,
        credential_id: &str,
        chat_id: &str,
        token: &str,
    ) -> Result<std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<WsMessage>> + Send>>> {
        let ws_url = self
            .base_url
            .replacen("http://", "ws://", 1)
            .replacen("https://", "wss://", 1);
        let url = format!("{}/ws/chat/{}/{}", ws_url, credential_id, chat_id);

        let request = tungstenite::http::Request::builder()
            .uri(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Host", extract_host(&self.base_url))
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tungstenite::handshake::client::generate_key(),
            )
            .body(())
            .context("Failed to build WebSocket request")?;

        let (ws_stream, _resp) = tokio_tungstenite::connect_async(request)
            .await
            .context("Failed to connect WebSocket")?;

        let mapped = ws_stream.filter_map(|msg| async {
            match msg {
                Ok(tungstenite::Message::Text(text)) => {
                    match serde_json::from_str::<WsMessage>(&text) {
                        Ok(ws_msg) => Some(Ok(ws_msg)),
                        Err(e) => Some(Err(anyhow::anyhow!("Failed to parse WS message: {}", e))),
                    }
                }
                Ok(tungstenite::Message::Close(_)) => None,
                Ok(_) => None, // Ping/Pong handled by tungstenite
                Err(e) => Some(Err(anyhow::anyhow!("WebSocket error: {}", e))),
            }
        });

        Ok(Box::pin(mapped))
    }

    // -- Admin (uses admin token) -------------------------------------------

    pub async fn health(&self) -> Result<HealthResponse> {
        let url = format!("{}/health", self.base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .context("Failed to connect to gateway")?;

        if !resp.status().is_success() {
            let status = resp.status();
            bail!("Health check failed: {}", status);
        }

        resp.json::<HealthResponse>()
            .await
            .context("Failed to parse health response")
    }

    pub async fn list_credentials(&self, admin_token: &str) -> Result<Vec<CredentialInfo>> {
        let url = format!("{}/admin/credentials", self.base_url);
        let resp = self
            .http
            .get(&url)
            .header(header::AUTHORIZATION, format!("Bearer {}", admin_token))
            .send()
            .await
            .context("Failed to connect to gateway")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to list credentials: {} — {}", status, body);
        }

        let list = resp
            .json::<CredentialListResponse>()
            .await
            .context("Failed to parse credentials response")?;

        Ok(list.credentials)
    }

    pub async fn create_credential(
        &self,
        admin_token: &str,
        req: &CreateCredentialRequest,
    ) -> Result<serde_json::Value> {
        let url = format!("{}/admin/credentials", self.base_url);
        let resp = self
            .http
            .post(&url)
            .header(header::AUTHORIZATION, format!("Bearer {}", admin_token))
            .json(req)
            .send()
            .await
            .context("Failed to connect to gateway")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to create credential: {} — {}", status, body);
        }

        resp.json::<serde_json::Value>()
            .await
            .context("Failed to parse create response")
    }

    pub async fn activate_credential(
        &self,
        admin_token: &str,
        id: &str,
    ) -> Result<serde_json::Value> {
        let url = format!("{}/admin/credentials/{}/activate", self.base_url, id);
        let resp = self
            .http
            .patch(&url)
            .header(header::AUTHORIZATION, format!("Bearer {}", admin_token))
            .send()
            .await
            .context("Failed to connect to gateway")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to activate credential: {} — {}", status, body);
        }

        resp.json::<serde_json::Value>()
            .await
            .context("Failed to parse response")
    }

    pub async fn deactivate_credential(
        &self,
        admin_token: &str,
        id: &str,
    ) -> Result<serde_json::Value> {
        let url = format!("{}/admin/credentials/{}/deactivate", self.base_url, id);
        let resp = self
            .http
            .patch(&url)
            .header(header::AUTHORIZATION, format!("Bearer {}", admin_token))
            .send()
            .await
            .context("Failed to connect to gateway")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to deactivate credential: {} — {}", status, body);
        }

        resp.json::<serde_json::Value>()
            .await
            .context("Failed to parse response")
    }
}

fn extract_host(url: &str) -> String {
    url.trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or("localhost")
        .to_string()
}
