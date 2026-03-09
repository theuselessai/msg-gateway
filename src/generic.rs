use axum::{
    Json,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::header,
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};

use crate::error::AppError;
use crate::guardrail::GuardrailVerdict;
use crate::message::{InboundMessage, MessageSource, UserInfo, WsOutboundMessage};
use crate::server::AppState;

/// Registry of active WebSocket connections
/// Key: (credential_id, chat_id) → broadcast sender for that chat
pub type WsRegistry = Arc<RwLock<HashMap<(String, String), broadcast::Sender<WsOutboundMessage>>>>;

/// Create a new WebSocket registry
pub fn new_ws_registry() -> WsRegistry {
    Arc::new(RwLock::new(HashMap::new()))
}

/// Request body for generic chat inbound
#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub chat_id: String,
    pub text: String,
    pub from: ChatUser,
    #[serde(default)]
    pub files: Vec<InboundFileRef>,
}

#[derive(Debug, Deserialize)]
pub struct ChatUser {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InboundFileRef {
    pub url: String,
    pub filename: String,
    pub mime_type: String,
    #[serde(default)]
    pub auth_header: Option<String>,
}

/// Response for chat inbound
#[derive(Debug, Serialize)]
pub struct ChatResponse {
    pub message_id: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// POST /api/v1/chat/{credential_id}
/// Generic adapter inbound - fire and forget
pub async fn chat_inbound(
    State(state): State<Arc<AppState>>,
    Path(credential_id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<ChatRequest>,
) -> Result<impl IntoResponse, AppError> {
    let config = state.config.read().await;

    // Find credential
    let credential = config
        .credentials
        .get(&credential_id)
        .ok_or_else(|| AppError::CredentialNotFound(credential_id.clone()))?;

    // Verify it's a generic adapter credential
    if credential.adapter != "generic" {
        return Err(AppError::Internal(format!(
            "Credential {} is not a generic adapter credential",
            credential_id
        )));
    }

    // Verify token
    let expected_token = &credential.token;
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(auth) if auth.starts_with("Bearer ") => {
            let token = &auth[7..];
            if token != expected_token {
                return Err(AppError::Unauthorized);
            }
        }
        _ => return Err(AppError::Unauthorized),
    }

    // Check if credential is active
    if !credential.active {
        return Err(AppError::CredentialInactive(credential_id.clone()));
    }

    let route = credential.route.clone();

    let backend_name = crate::backend::resolve_backend_name(credential, &config.gateway)
        .ok_or_else(|| {
            AppError::Internal("No backend configured for this credential".to_string())
        })?;
    let backend_cfg = config.backends.get(&backend_name).ok_or_else(|| {
        AppError::Internal(format!("Backend '{}' not found in config", backend_name))
    })?;
    let gateway_ctx = crate::backend::GatewayContext {
        gateway_url: format!("http://{}", config.gateway.listen),
        send_token: config.auth.send_token.clone(),
    };
    let adapter = match crate::backend::create_adapter(
        backend_cfg,
        Some(&gateway_ctx),
        credential.config.as_ref().or(backend_cfg.config.as_ref()),
    ) {
        Ok(a) => a,
        Err(e) => {
            return Err(AppError::Internal(format!(
                "Failed to create backend adapter: {}",
                e
            )));
        }
    };
    drop(config);

    // Generate message ID
    let message_id = format!("generic_{}", uuid::Uuid::new_v4());
    let timestamp = chrono::Utc::now();

    // Download and cache file attachments
    let mut attachments = vec![];

    // Warn once if files present but cache not configured
    if state.file_cache.is_none() && !payload.files.is_empty() {
        tracing::warn!("Files received but file cache not configured, skipping attachments");
    }

    for file_ref in &payload.files {
        let Some(ref file_cache) = state.file_cache else {
            continue;
        };

        match file_cache
            .download_and_cache(
                &file_ref.url,
                file_ref.auth_header.as_deref(),
                &file_ref.filename,
                &file_ref.mime_type,
            )
            .await
        {
            Ok(cached) => {
                attachments.push(crate::message::Attachment {
                    filename: cached.filename.clone(),
                    mime_type: cached.mime_type.clone(),
                    size_bytes: cached.size_bytes,
                    download_url: file_cache.get_download_url(&cached.file_id),
                });
                tracing::info!(
                    file_id = %cached.file_id,
                    filename = %cached.filename,
                    "Generic inbound file cached"
                );
            }
            Err(e) => {
                tracing::warn!(
                    url = %file_ref.url,
                    error = %e,
                    "Failed to cache generic inbound file attachment"
                );
            }
        }
    }

    // Build normalized inbound message
    let inbound = InboundMessage {
        route,
        credential_id: credential_id.clone(),
        source: MessageSource {
            protocol: "generic".to_string(),
            chat_id: payload.chat_id.clone(),
            message_id: message_id.clone(),
            reply_to_message_id: None,
            from: UserInfo {
                id: payload.from.id,
                username: None,
                display_name: payload.from.display_name,
            },
        },
        text: payload.text,
        attachments,
        timestamp,
        extra_data: None,
    };

    let verdict = {
        let engine = state.guardrail_engine.read().await;
        engine.evaluate_inbound(&inbound)
    };
    match verdict {
        GuardrailVerdict::Block { reject_message, .. } => {
            return Err(AppError::Forbidden(reject_message));
        }
        GuardrailVerdict::Allow => {}
    }

    // Check if target server is down - buffer message instead of forwarding
    let health_state = state.health_monitor.get_state().await;
    if health_state == crate::health::HealthState::Down {
        state.health_monitor.buffer_message(inbound).await;
        tracing::info!(
            credential_id = %credential_id,
            message_id = %message_id,
            "Message buffered (target server down)"
        );
    } else {
        // Clone for the spawned task
        let message_id_for_task = message_id.clone();
        let credential_id_for_task = credential_id.clone();

        // Forward to backend (fire and forget - spawn task)
        tokio::spawn(async move {
            match adapter.send_message(&inbound).await {
                Ok(()) => {
                    tracing::debug!(
                        credential_id = %credential_id_for_task,
                        message_id = %message_id_for_task,
                        "Message forwarded to backend"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        credential_id = %credential_id_for_task,
                        error = %e,
                        "Failed to forward message to backend"
                    );
                }
            }
        });
    }

    // Return immediately (fire and forget)
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(ChatResponse {
            message_id,
            timestamp,
        }),
    ))
}

/// GET /ws/chat/{credential_id}/{chat_id}
/// WebSocket upgrade for outbound messages
pub async fn ws_handler(
    State(state): State<Arc<AppState>>,
    Path((credential_id, chat_id)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, AppError> {
    let config = state.config.read().await;

    // Find credential
    let credential = config
        .credentials
        .get(&credential_id)
        .ok_or_else(|| AppError::CredentialNotFound(credential_id.clone()))?;

    // Verify it's a generic adapter credential
    if credential.adapter != "generic" {
        return Err(AppError::Internal(format!(
            "Credential {} is not a generic adapter credential",
            credential_id
        )));
    }

    // Verify token
    let expected_token = &credential.token;
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(auth) if auth.starts_with("Bearer ") => {
            let token = &auth[7..];
            if token != expected_token {
                return Err(AppError::Unauthorized);
            }
        }
        _ => return Err(AppError::Unauthorized),
    }

    if !credential.active {
        return Err(AppError::CredentialInactive(credential_id.clone()));
    }

    drop(config);

    let ws_registry = state.ws_registry.clone();
    let cred_id = credential_id.clone();
    let c_id = chat_id.clone();

    Ok(ws.on_upgrade(move |socket| handle_ws(socket, ws_registry, cred_id, c_id)))
}

async fn handle_ws(
    socket: WebSocket,
    registry: WsRegistry,
    credential_id: String,
    chat_id: String,
) {
    let (sender, mut receiver) = socket.split();
    let sender = Arc::new(tokio::sync::Mutex::new(sender));

    // Create or get broadcast channel for this chat
    let mut rx = {
        let mut reg = registry.write().await;
        let key = (credential_id.clone(), chat_id.clone());

        let tx = reg.entry(key).or_insert_with(|| {
            let (tx, _) = broadcast::channel(100);
            tx
        });

        tx.subscribe()
    };

    tracing::info!(
        credential_id = %credential_id,
        chat_id = %chat_id,
        "WebSocket connected"
    );

    // Spawn task to receive messages from broadcast and send to WebSocket
    let sender_clone = sender.clone();
    let send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            let json = serde_json::to_string(&msg).unwrap();
            let mut s = sender_clone.lock().await;
            if s.send(Message::Text(json.into())).await.is_err() {
                break;
            }
        }
    });

    // Handle incoming WebSocket messages (for ping/pong, close, etc.)
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) => {
                // Pong is handled automatically by axum
                tracing::trace!("Received ping");
            }
            Err(e) => {
                tracing::debug!(error = %e, "WebSocket error");
                break;
            }
            _ => {}
        }
    }

    // Cleanup
    send_task.abort();

    // Remove from registry if no more subscribers
    {
        let mut reg = registry.write().await;
        let key = (credential_id.clone(), chat_id.clone());
        if let Some(tx) = reg.get(&key)
            && tx.receiver_count() == 0
        {
            reg.remove(&key);
        }
    }

    tracing::info!(
        credential_id = %credential_id,
        chat_id = %chat_id,
        "WebSocket disconnected"
    );
}

/// Send a message to a WebSocket client (called from /api/v1/send)
pub async fn send_to_ws(
    registry: &WsRegistry,
    credential_id: &str,
    chat_id: &str,
    message: WsOutboundMessage,
) -> bool {
    let reg = registry.read().await;
    let key = (credential_id.to_string(), chat_id.to_string());

    if let Some(tx) = reg.get(&key) {
        match tx.send(message) {
            Ok(_) => {
                tracing::debug!(
                    credential_id = %credential_id,
                    chat_id = %chat_id,
                    "Message sent to WebSocket"
                );
                true
            }
            Err(_) => {
                tracing::debug!(
                    credential_id = %credential_id,
                    chat_id = %chat_id,
                    "No active WebSocket subscribers"
                );
                false
            }
        }
    } else {
        tracing::debug!(
            credential_id = %credential_id,
            chat_id = %chat_id,
            "No WebSocket connection for this chat"
        );
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ws_message(text: &str, message_id: &str) -> WsOutboundMessage {
        WsOutboundMessage {
            text: text.to_string(),
            timestamp: chrono::Utc::now(),
            message_id: message_id.to_string(),
            file_urls: vec![],
        }
    }

    // ==================== WsRegistry Tests ====================

    #[test]
    fn test_new_ws_registry() {
        let registry = new_ws_registry();
        // Should be empty
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let reg = registry.read().await;
            assert!(reg.is_empty());
        });
    }

    #[tokio::test]
    async fn test_ws_registry_add_channel() {
        let registry = new_ws_registry();

        // Add a channel
        {
            let mut reg = registry.write().await;
            let (tx, _) = broadcast::channel::<WsOutboundMessage>(100);
            reg.insert(("cred1".to_string(), "chat1".to_string()), tx);
        }

        // Verify it exists
        {
            let reg = registry.read().await;
            assert!(reg.contains_key(&("cred1".to_string(), "chat1".to_string())));
        }
    }

    #[tokio::test]
    async fn test_ws_registry_remove_channel() {
        let registry = new_ws_registry();

        // Add and remove
        {
            let mut reg = registry.write().await;
            let (tx, _) = broadcast::channel::<WsOutboundMessage>(100);
            reg.insert(("cred1".to_string(), "chat1".to_string()), tx);
        }

        {
            let mut reg = registry.write().await;
            reg.remove(&("cred1".to_string(), "chat1".to_string()));
        }

        {
            let reg = registry.read().await;
            assert!(!reg.contains_key(&("cred1".to_string(), "chat1".to_string())));
        }
    }

    // ==================== send_to_ws Tests ====================

    #[tokio::test]
    async fn test_send_to_ws_no_connection() {
        let registry = new_ws_registry();

        let message = make_ws_message("Hello", "msg_123");

        let result = send_to_ws(&registry, "cred1", "chat1", message).await;
        assert!(!result);
    }

    #[tokio::test]
    async fn test_send_to_ws_with_subscriber() {
        let registry = new_ws_registry();

        // Add a channel with subscriber
        let mut rx = {
            let mut reg = registry.write().await;
            let (tx, rx) = broadcast::channel::<WsOutboundMessage>(100);
            reg.insert(("cred1".to_string(), "chat1".to_string()), tx);
            rx
        };

        let message = make_ws_message("Hello", "msg_123");

        let result = send_to_ws(&registry, "cred1", "chat1", message).await;
        assert!(result);

        // Verify message was received
        let received = rx.recv().await.unwrap();
        assert_eq!(received.text, "Hello");
        assert_eq!(received.message_id, "msg_123");
    }

    #[tokio::test]
    async fn test_send_to_ws_no_subscribers() {
        let registry = new_ws_registry();

        // Add a channel without keeping the receiver (so no subscribers)
        {
            let mut reg = registry.write().await;
            let (tx, _rx) = broadcast::channel::<WsOutboundMessage>(100);
            // Drop the receiver immediately
            drop(_rx);
            reg.insert(("cred1".to_string(), "chat1".to_string()), tx);
        }

        let message = make_ws_message("Hello", "msg_456");

        let result = send_to_ws(&registry, "cred1", "chat1", message).await;
        // Should return false because no active subscribers
        assert!(!result);
    }

    // ==================== ChatRequest Tests ====================

    #[test]
    fn test_chat_request_parse() {
        let json = r#"{
            "chat_id": "12345",
            "text": "Hello, world!",
            "from": {
                "id": "user_1",
                "display_name": "Test User"
            }
        }"#;

        let req: ChatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.chat_id, "12345");
        assert_eq!(req.text, "Hello, world!");
        assert_eq!(req.from.id, "user_1");
        assert_eq!(req.from.display_name, Some("Test User".to_string()));
    }

    #[test]
    fn test_chat_request_minimal() {
        let json = r#"{
            "chat_id": "12345",
            "text": "Hello",
            "from": {"id": "user_1"}
        }"#;

        let req: ChatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.chat_id, "12345");
        assert_eq!(req.from.id, "user_1");
        assert!(req.from.display_name.is_none());
    }

    // ==================== ChatUser Tests ====================

    #[test]
    fn test_chat_user_parse() {
        let json = r#"{"id": "user_123", "display_name": "John Doe"}"#;
        let user: ChatUser = serde_json::from_str(json).unwrap();
        assert_eq!(user.id, "user_123");
        assert_eq!(user.display_name, Some("John Doe".to_string()));
    }

    #[test]
    fn test_chat_user_minimal() {
        let json = r#"{"id": "user_123"}"#;
        let user: ChatUser = serde_json::from_str(json).unwrap();
        assert_eq!(user.id, "user_123");
        assert!(user.display_name.is_none());
    }

    // ==================== ChatResponse Tests ====================

    #[test]
    fn test_chat_response_serialize() {
        let response = ChatResponse {
            message_id: "msg_123".to_string(),
            timestamp: chrono::Utc::now(),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"message_id\":\"msg_123\""));
        assert!(json.contains("\"timestamp\""));
    }

    // ==================== Multiple Channels Tests ====================

    #[tokio::test]
    async fn test_ws_registry_multiple_chats() {
        let registry = new_ws_registry();

        // Add multiple channels
        let mut rx1;
        let mut rx2;
        {
            let mut reg = registry.write().await;
            let (tx1, r1) = broadcast::channel::<WsOutboundMessage>(100);
            let (tx2, r2) = broadcast::channel::<WsOutboundMessage>(100);
            reg.insert(("cred1".to_string(), "chat1".to_string()), tx1);
            reg.insert(("cred1".to_string(), "chat2".to_string()), tx2);
            rx1 = r1;
            rx2 = r2;
        }

        // Send to chat1
        let msg1 = make_ws_message("Message for chat1", "msg_1");
        assert!(send_to_ws(&registry, "cred1", "chat1", msg1).await);

        // Send to chat2
        let msg2 = make_ws_message("Message for chat2", "msg_2");
        assert!(send_to_ws(&registry, "cred1", "chat2", msg2).await);

        // Verify correct routing
        let received1 = rx1.recv().await.unwrap();
        assert_eq!(received1.text, "Message for chat1");

        let received2 = rx2.recv().await.unwrap();
        assert_eq!(received2.text, "Message for chat2");
    }

    #[tokio::test]
    async fn test_ws_registry_different_credentials() {
        let registry = new_ws_registry();

        // Add channels for different credentials
        let mut rx_cred1;
        let mut rx_cred2;
        {
            let mut reg = registry.write().await;
            let (tx1, r1) = broadcast::channel::<WsOutboundMessage>(100);
            let (tx2, r2) = broadcast::channel::<WsOutboundMessage>(100);
            reg.insert(("cred1".to_string(), "chat".to_string()), tx1);
            reg.insert(("cred2".to_string(), "chat".to_string()), tx2);
            rx_cred1 = r1;
            rx_cred2 = r2;
        }

        // Send to cred1
        let msg = make_ws_message("For cred1", "msg_1");
        assert!(send_to_ws(&registry, "cred1", "chat", msg).await);

        // Only cred1 should receive
        let received = rx_cred1.recv().await.unwrap();
        assert_eq!(received.text, "For cred1");

        // cred2 should not have received (try_recv returns error)
        assert!(rx_cred2.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_send_to_ws_multiple_messages() {
        let registry = new_ws_registry();

        let mut rx = {
            let mut reg = registry.write().await;
            let (tx, rx) = broadcast::channel::<WsOutboundMessage>(100);
            reg.insert(("cred1".to_string(), "chat1".to_string()), tx);
            rx
        };

        // Send multiple messages
        for i in 1..=5 {
            let message = make_ws_message(&format!("Message {}", i), &format!("msg_{}", i));
            let result = send_to_ws(&registry, "cred1", "chat1", message).await;
            assert!(result);
        }

        // Verify all messages received in order
        for i in 1..=5 {
            let received = rx.recv().await.unwrap();
            assert_eq!(received.text, format!("Message {}", i));
            assert_eq!(received.message_id, format!("msg_{}", i));
        }
    }

    #[tokio::test]
    async fn test_ws_registry_broadcast_to_multiple_subscribers() {
        let registry = new_ws_registry();

        // Add a channel with multiple subscribers
        let (mut rx1, mut rx2);
        {
            let mut reg = registry.write().await;
            let (tx, r1) = broadcast::channel::<WsOutboundMessage>(100);
            rx1 = r1;
            rx2 = tx.subscribe();
            reg.insert(("cred1".to_string(), "chat1".to_string()), tx);
        }

        let message = make_ws_message("Broadcast message", "msg_broadcast");
        let result = send_to_ws(&registry, "cred1", "chat1", message).await;
        assert!(result);

        // Both subscribers should receive the message
        let received1 = rx1.recv().await.unwrap();
        let received2 = rx2.recv().await.unwrap();

        assert_eq!(received1.text, "Broadcast message");
        assert_eq!(received2.text, "Broadcast message");
    }

    #[test]
    fn test_chat_request_with_files() {
        let json = r#"{
            "chat_id": "123",
            "text": "see attached",
            "from": {"id": "u1"},
            "files": [
                {"url": "https://example.com/img.jpg", "filename": "img.jpg", "mime_type": "image/jpeg"}
            ]
        }"#;
        let req: ChatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.files.len(), 1);
        assert_eq!(req.files[0].url, "https://example.com/img.jpg");
        assert_eq!(req.files[0].filename, "img.jpg");
        assert!(req.files[0].auth_header.is_none());
    }

    #[test]
    fn test_chat_request_no_files() {
        let json = r#"{"chat_id": "123", "text": "hello", "from": {"id": "u1"}}"#;
        let req: ChatRequest = serde_json::from_str(json).unwrap();
        assert!(req.files.is_empty());
    }
}
