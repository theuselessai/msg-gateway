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
}

#[derive(Debug, Deserialize)]
pub struct ChatUser {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
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

    // Resolve target for this credential
    let target = crate::backend::resolve_target(credential, &config.gateway.default_target);
    let adapter = match crate::backend::create_adapter(target) {
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

    // Build normalized inbound message
    let inbound = InboundMessage {
        route,
        credential_id: credential_id.clone(),
        source: MessageSource {
            protocol: "generic".to_string(),
            chat_id: payload.chat_id.clone(),
            message_id: message_id.clone(),
            from: UserInfo {
                id: payload.from.id,
                username: None,
                display_name: payload.from.display_name,
            },
        },
        text: payload.text,
        attachments: vec![],
        timestamp,
    };

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
