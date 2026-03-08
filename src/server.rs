use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{Request, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::trace::TraceLayer;

use crate::adapter::AdapterInstanceManager;
use crate::admin;
use crate::config::Config;
use crate::error::AppError;
use crate::files::FileCache;
use crate::generic::{self, WsRegistry};
use crate::health::HealthMonitor;
use crate::manager::CredentialManager;
use crate::message::WsOutboundMessage;

pub struct AppState {
    pub config: RwLock<Config>,
    pub ws_registry: WsRegistry,
    pub manager: Arc<CredentialManager>,
    pub adapter_manager: Arc<AdapterInstanceManager>,
    /// Timestamp until which config reload should be skipped (Admin API writes)
    pub skip_reload_until: RwLock<Option<std::time::Instant>>,
    /// Health monitor for emergency mode
    pub health_monitor: HealthMonitor,
    /// File cache (optional, None if file_cache not configured)
    pub file_cache: Option<Arc<FileCache>>,
}

use std::future::Future;
use std::pin::Pin;

/// Create the server and return the state + a future to run
pub async fn create_server(
    config: Config,
    manager: Arc<CredentialManager>,
    adapter_manager: Arc<AdapterInstanceManager>,
) -> anyhow::Result<(
    Arc<AppState>,
    Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>,
)> {
    let listen_addr = config.gateway.listen.clone();
    let gateway_url = format!("http://{}", listen_addr);

    // Default max buffer size: 1000 messages
    let max_buffer_size = 1000;

    // Initialize file cache if configured
    let file_cache = if let Some(ref cache_config) = config.gateway.file_cache {
        match FileCache::new(cache_config.clone(), &gateway_url).await {
            Ok(cache) => {
                tracing::info!(
                    directory = %cache_config.directory,
                    "File cache initialized"
                );
                Some(Arc::new(cache))
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to initialize file cache");
                None
            }
        }
    } else {
        None
    };

    let state = Arc::new(AppState {
        config: RwLock::new(config),
        ws_registry: generic::new_ws_registry(),
        manager,
        adapter_manager,
        skip_reload_until: RwLock::new(None),
        health_monitor: HealthMonitor::new(max_buffer_size),
        file_cache,
    });

    let app = Router::new()
        // Public health endpoint (no auth)
        .route("/health", get(health))
        // Send endpoint (requires send_token)
        .route("/api/v1/send", post(send_message))
        // Adapter inbound endpoint (from external adapters)
        .route("/api/v1/adapter/inbound", post(adapter_inbound))
        // File serving endpoint (requires send_token)
        .route("/files/{file_id}", get(serve_file))
        // Generic protocol endpoints
        .route("/api/v1/chat/{credential_id}", post(generic::chat_inbound))
        .route(
            "/ws/chat/{credential_id}/{chat_id}",
            get(generic::ws_handler),
        )
        // Admin routes (requires admin_token)
        .nest("/admin", admin_routes(state.clone()))
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(&listen_addr).await?;
    tracing::info!("Listening on {}", listen_addr);

    let server_future = Box::pin(async move {
        axum::serve(listener, app).await?;
        Ok(())
    });

    Ok((state, server_future))
}

fn admin_routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    use axum::routing::patch;

    Router::new()
        .route("/health", get(admin_health))
        .route(
            "/credentials",
            get(list_credentials).post(admin::create_credential),
        )
        .route(
            "/credentials/{id}",
            get(admin::get_credential)
                .put(admin::update_credential)
                .delete(admin::delete_credential),
        )
        .route(
            "/credentials/{id}/activate",
            patch(admin::activate_credential),
        )
        .route(
            "/credentials/{id}/deactivate",
            patch(admin::deactivate_credential),
        )
        .layer(middleware::from_fn_with_state(state, admin_auth_middleware))
}

// Middleware for admin authentication
async fn admin_auth_middleware(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    let config = state.config.read().await;
    let expected_token = &config.gateway.admin_token;

    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(auth) if auth.starts_with("Bearer ") => {
            let token = &auth[7..];
            if token == expected_token {
                drop(config);
                Ok(next.run(request).await)
            } else {
                Err(AppError::Unauthorized)
            }
        }
        _ => Err(AppError::Unauthorized),
    }
}

// Public health check
async fn health() -> impl IntoResponse {
    Json(json!({
        "status": "ok"
    }))
}

// Admin health check with more details
async fn admin_health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config.read().await;
    let credential_count = config.credentials.len();
    let active_count = config.credentials.values().filter(|c| c.active).count();
    drop(config);

    // Get instance statuses
    let instance_statuses = state.manager.registry.get_all_status().await;
    let running_count = instance_statuses
        .values()
        .filter(|(_, status)| *status == crate::manager::InstanceStatus::Running)
        .count();

    // Get adapter health statuses
    let adapter_health = state.adapter_manager.get_all_health().await;
    let adapters: Vec<_> = adapter_health
        .iter()
        .map(|(cred_id, (adapter_name, health, failures))| {
            json!({
                "credential_id": cred_id,
                "adapter": adapter_name,
                "health": format!("{:?}", health),
                "consecutive_failures": failures
            })
        })
        .collect();

    // Get health monitor status
    let health_state = state.health_monitor.get_state().await;
    let buffer_size = state.health_monitor.buffer_size().await;
    let last_healthy = state
        .health_monitor
        .last_healthy_ago()
        .await
        .map(|d| format!("{:.1}s ago", d.as_secs_f64()));

    Json(json!({
        "status": "ok",
        "credentials": {
            "total": credential_count,
            "active": active_count,
            "running_tasks": running_count
        },
        "adapters": adapters,
        "target_server": {
            "state": health_state.to_string(),
            "last_healthy": last_healthy,
            "buffered_messages": buffer_size
        }
    }))
}

// List all credentials (tokens redacted)
async fn list_credentials(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config.read().await;

    let credentials: Vec<_> = config
        .credentials
        .iter()
        .map(|(id, cred)| {
            json!({
                "id": id,
                "adapter": cred.adapter,
                "active": cred.active,
                "emergency": cred.emergency,
                "route": cred.route
            })
        })
        .collect();

    Json(json!({
        "credentials": credentials
    }))
}

/// File attachment in send request
#[derive(Debug, serde::Deserialize)]
struct SendFileAttachment {
    /// URL to download the file from
    url: String,
    /// Original filename
    filename: String,
    /// MIME type
    mime_type: String,
    /// Optional auth header for downloading
    #[serde(default)]
    auth_header: Option<String>,
}

// Send message endpoint (Pipelit → Gateway → Protocol)
async fn send_message(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    // Verify send token
    let config = state.config.read().await;
    let expected_token = &config.auth.send_token;

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

    // Extract fields from payload
    let credential_id = payload
        .get("credential_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Internal("Missing credential_id".to_string()))?;

    let chat_id = payload
        .get("chat_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Internal("Missing chat_id".to_string()))?;

    let text = payload.get("text").and_then(|v| v.as_str()).unwrap_or(""); // Text is optional when sending file

    // Parse optional file attachment
    let file_attachment: Option<SendFileAttachment> = payload
        .get("file")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    // Check credential exists and is active
    let credential = config
        .credentials
        .get(credential_id)
        .ok_or_else(|| AppError::CredentialNotFound(credential_id.to_string()))?;

    if !credential.active {
        return Err(AppError::CredentialInactive(credential_id.to_string()));
    }

    let adapter = credential.adapter.clone();
    drop(config);

    let message_id = format!("{}_{}", adapter, uuid::Uuid::new_v4());
    let timestamp = chrono::Utc::now();

    // Handle file attachment: download and cache
    let file_path: Option<String> = if let Some(file) = file_attachment {
        if let Some(ref file_cache) = state.file_cache {
            match file_cache
                .download_and_cache(
                    &file.url,
                    file.auth_header.as_deref(),
                    &file.filename,
                    &file.mime_type,
                )
                .await
            {
                Ok(cached) => {
                    tracing::info!(
                        file_id = %cached.file_id,
                        filename = %file.filename,
                        "Outbound file cached"
                    );
                    Some(cached.path.to_string_lossy().to_string())
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        filename = %file.filename,
                        "Failed to cache outbound file"
                    );
                    return Err(AppError::Internal(format!(
                        "Failed to download file: {}",
                        e
                    )));
                }
            }
        } else {
            tracing::warn!("File attachment in send request but file cache not configured");
            return Err(AppError::Internal("File cache not configured".to_string()));
        }
    } else {
        None
    };

    // Route to appropriate adapter
    if adapter == "generic" {
        // Built-in generic adapter: send via WebSocket
        let ws_msg = WsOutboundMessage {
            text: text.to_string(),
            timestamp,
            message_id: message_id.clone(),
        };

        let sent = generic::send_to_ws(&state.ws_registry, credential_id, chat_id, ws_msg).await;

        if sent {
            tracing::info!(
                credential_id = credential_id,
                chat_id = chat_id,
                "Message sent via WebSocket"
            );
        } else {
            tracing::warn!(
                credential_id = credential_id,
                chat_id = chat_id,
                "No WebSocket connection, message dropped"
            );
        }
    } else {
        // External adapter: POST to adapter's /send endpoint
        let port = state.adapter_manager.get_port(credential_id).await;

        match port {
            Some(port) if port > 0 => {
                let send_req = crate::adapter::AdapterSendRequest {
                    chat_id: chat_id.to_string(),
                    text: text.to_string(),
                    reply_to_message_id: payload
                        .get("reply_to_message_id")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    file_path: file_path.clone(),
                };

                let client = reqwest::Client::new();
                let url = format!("http://127.0.0.1:{}/send", port);

                match client.post(&url).json(&send_req).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        match resp.json::<crate::adapter::AdapterSendResponse>().await {
                            Ok(adapter_resp) => {
                                tracing::info!(
                                    credential_id = credential_id,
                                    adapter = adapter,
                                    protocol_message_id = %adapter_resp.protocol_message_id,
                                    "Message sent via adapter"
                                );
                                return Ok(Json(json!({
                                    "status": "sent",
                                    "protocol_message_id": adapter_resp.protocol_message_id,
                                    "timestamp": timestamp.to_rfc3339()
                                })));
                            }
                            Err(e) => {
                                tracing::error!(
                                    credential_id = credential_id,
                                    error = %e,
                                    "Failed to parse adapter response"
                                );
                            }
                        }
                    }
                    Ok(resp) => {
                        tracing::error!(
                            credential_id = credential_id,
                            status = %resp.status(),
                            "Adapter returned error"
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            credential_id = credential_id,
                            error = %e,
                            "Failed to send to adapter"
                        );
                    }
                }
            }
            _ => {
                tracing::warn!(
                    credential_id = credential_id,
                    adapter = adapter,
                    "No adapter instance running for credential"
                );
            }
        }
    }

    Ok(Json(json!({
        "status": "sent",
        "protocol_message_id": message_id,
        "timestamp": timestamp.to_rfc3339()
    })))
}

// Inbound message from external adapter
async fn adapter_inbound(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<crate::adapter::AdapterInboundRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Look up credential by instance_id
    let credential_id = state
        .adapter_manager
        .get_credential_id(&payload.instance_id)
        .await
        .ok_or_else(|| {
            tracing::warn!(
                instance_id = %payload.instance_id,
                "Could not find credential for instance"
            );
            AppError::Internal(format!("Unknown instance: {}", payload.instance_id))
        })?;

    let config = state.config.read().await;

    let credential = config
        .credentials
        .get(&credential_id)
        .ok_or_else(|| AppError::CredentialNotFound(credential_id.clone()))?;

    if !credential.active {
        return Err(AppError::CredentialInactive(credential_id.clone()));
    }

    let route = credential.route.clone();
    let adapter = credential.adapter.clone();

    // Resolve target for this credential
    let target = crate::backend::resolve_target(credential, &config.gateway.default_target);
    let backend_adapter = crate::backend::create_adapter(target)
        .map_err(|e| AppError::Internal(format!("Failed to create backend adapter: {}", e)))?;
    drop(config);

    // Build normalized inbound message
    let timestamp = payload
        .timestamp
        .as_ref()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);

    // Handle file attachment if present
    let mut attachments = vec![];
    if let Some(ref file_info) = payload.file {
        if let Some(ref file_cache) = state.file_cache {
            match file_cache
                .download_and_cache(
                    &file_info.url,
                    file_info.auth_header.as_deref(),
                    &file_info.filename,
                    &file_info.mime_type,
                )
                .await
            {
                Ok(cached) => {
                    attachments.push(crate::message::Attachment {
                        filename: cached.filename,
                        mime_type: cached.mime_type,
                        size_bytes: cached.size_bytes,
                        download_url: file_cache.get_download_url(&cached.file_id),
                    });
                    tracing::info!(
                        file_id = %cached.file_id,
                        filename = %file_info.filename,
                        "File attachment cached"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        filename = %file_info.filename,
                        "Failed to cache file attachment"
                    );
                    // Include a stub attachment with error info
                    attachments.push(crate::message::Attachment {
                        filename: file_info.filename.clone(),
                        mime_type: file_info.mime_type.clone(),
                        size_bytes: 0,
                        download_url: format!("error: {}", e),
                    });
                }
            }
        } else {
            tracing::warn!("File attachment received but file cache not configured");
        }
    }

    let inbound = crate::message::InboundMessage {
        route,
        credential_id: credential_id.clone(),
        source: crate::message::MessageSource {
            protocol: adapter.clone(),
            chat_id: payload.chat_id.clone(),
            message_id: payload.message_id.clone(),
            from: crate::message::UserInfo {
                id: payload.from.id,
                username: payload.from.username,
                display_name: payload.from.display_name,
            },
        },
        text: payload.text,
        attachments,
        timestamp,
    };

    // Check health state
    let health_state = state.health_monitor.get_state().await;
    if health_state == crate::health::HealthState::Down {
        state.health_monitor.buffer_message(inbound).await;
        tracing::info!(
            credential_id = %credential_id,
            instance_id = %payload.instance_id,
            "Message buffered (target server down)"
        );
    } else {
        // Forward to backend
        let instance_id = payload.instance_id.clone();
        let cred_id = credential_id.clone();

        tokio::spawn(async move {
            match backend_adapter.send_message(&inbound).await {
                Ok(()) => {
                    tracing::debug!(
                        credential_id = %cred_id,
                        instance_id = %instance_id,
                        "Message forwarded to backend"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        credential_id = %cred_id,
                        instance_id = %instance_id,
                        error = %e,
                        "Failed to forward message to backend"
                    );
                }
            }
        });
    }

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({
            "status": "accepted"
        })),
    ))
}

// Serve cached files
async fn serve_file(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(file_id): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    // Verify send token (same as /api/v1/send)
    let config = state.config.read().await;
    let expected_token = &config.auth.send_token;

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
    drop(config);

    // Get file cache
    let file_cache = state
        .file_cache
        .as_ref()
        .ok_or_else(|| AppError::Internal("File cache not configured".to_string()))?;

    // Get file metadata
    let cached = file_cache
        .get(&file_id)
        .await
        .ok_or_else(|| AppError::NotFound(format!("File not found: {}", file_id)))?;

    // Read file content
    let content = file_cache.read_file(&file_id).await?;

    // Build response with appropriate headers
    let content_disposition = format!(
        "attachment; filename=\"{}\"",
        cached.filename.replace("\"", "\\\"")
    );

    Ok((
        [
            (header::CONTENT_TYPE, cached.mime_type),
            (header::CONTENT_DISPOSITION, content_disposition),
        ],
        content,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== SendFileAttachment Tests ====================

    #[test]
    fn test_send_file_attachment_parse() {
        let json = r#"{
            "url": "https://example.com/file.pdf",
            "filename": "document.pdf",
            "mime_type": "application/pdf"
        }"#;

        let attachment: SendFileAttachment = serde_json::from_str(json).unwrap();
        assert_eq!(attachment.url, "https://example.com/file.pdf");
        assert_eq!(attachment.filename, "document.pdf");
        assert_eq!(attachment.mime_type, "application/pdf");
        assert!(attachment.auth_header.is_none());
    }

    #[test]
    fn test_send_file_attachment_with_auth() {
        let json = r#"{
            "url": "https://example.com/file.pdf",
            "filename": "document.pdf",
            "mime_type": "application/pdf",
            "auth_header": "Bearer token123"
        }"#;

        let attachment: SendFileAttachment = serde_json::from_str(json).unwrap();
        assert_eq!(attachment.auth_header, Some("Bearer token123".to_string()));
    }

    // ==================== Content-Disposition Escaping Tests ====================

    #[test]
    fn test_content_disposition_escaping() {
        // Test filename with quotes
        let filename = r#"file"name.pdf"#;
        let content_disposition = format!(
            "attachment; filename=\"{}\"",
            filename.replace("\"", "\\\"")
        );
        assert_eq!(
            content_disposition,
            r#"attachment; filename="file\"name.pdf""#
        );
    }

    #[test]
    fn test_content_disposition_normal() {
        let filename = "document.pdf";
        let content_disposition = format!(
            "attachment; filename=\"{}\"",
            filename.replace("\"", "\\\"")
        );
        assert_eq!(
            content_disposition,
            r#"attachment; filename="document.pdf""#
        );
    }

    #[test]
    fn test_send_file_attachment_missing_optional() {
        // auth_header is optional
        let json = r#"{
            "url": "https://example.com/file.txt",
            "filename": "test.txt",
            "mime_type": "text/plain"
        }"#;

        let attachment: SendFileAttachment = serde_json::from_str(json).unwrap();
        assert!(attachment.auth_header.is_none());
    }

    #[test]
    fn test_send_file_attachment_debug() {
        let attachment = SendFileAttachment {
            url: "https://example.com/file.pdf".to_string(),
            filename: "doc.pdf".to_string(),
            mime_type: "application/pdf".to_string(),
            auth_header: None,
        };

        let debug_str = format!("{:?}", attachment);
        assert!(debug_str.contains("SendFileAttachment"));
        assert!(debug_str.contains("doc.pdf"));
    }

    #[test]
    fn test_content_disposition_special_chars() {
        // Test with special characters in filename
        let filename = "file with spaces.pdf";
        let content_disposition = format!(
            "attachment; filename=\"{}\"",
            filename.replace("\"", "\\\"")
        );
        assert_eq!(
            content_disposition,
            r#"attachment; filename="file with spaces.pdf""#
        );
    }

    #[test]
    fn test_content_disposition_unicode() {
        // Test with unicode characters
        let filename = "文档.pdf";
        let content_disposition = format!(
            "attachment; filename=\"{}\"",
            filename.replace("\"", "\\\"")
        );
        assert!(content_disposition.contains("文档.pdf"));
    }

    #[test]
    fn test_send_file_attachment_various_mime_types() {
        let test_cases = vec![
            ("image/png", "image.png"),
            ("video/mp4", "video.mp4"),
            ("audio/mpeg", "audio.mp3"),
            ("application/json", "data.json"),
            ("text/html", "page.html"),
        ];

        for (mime_type, filename) in test_cases {
            let json = format!(
                r#"{{"url": "https://example.com/{}", "filename": "{}", "mime_type": "{}"}}"#,
                filename, filename, mime_type
            );
            let attachment: SendFileAttachment = serde_json::from_str(&json).unwrap();
            assert_eq!(attachment.mime_type, mime_type);
            assert_eq!(attachment.filename, filename);
        }
    }
}
