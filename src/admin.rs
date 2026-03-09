//! Admin API endpoints for credential management
//!
//! All endpoints require admin_token authentication.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

use crate::config::CredentialConfig;
use crate::error::AppError;
use crate::server::AppState;

/// Request body for creating a credential
#[derive(Debug, Deserialize)]
pub struct CreateCredentialRequest {
    pub id: String,
    /// Adapter name (must exist in adapters_dir, or "generic" for built-in)
    pub adapter: String,
    pub token: String,
    #[serde(default = "default_true")]
    pub active: bool,
    #[serde(default)]
    pub emergency: bool,
    #[serde(default)]
    pub config: Option<serde_json::Value>,
    #[serde(default)]
    pub backend: Option<String>,
    pub route: serde_json::Value,
}

fn default_true() -> bool {
    true
}

/// Request body for updating a credential
#[derive(Debug, Deserialize)]
pub struct UpdateCredentialRequest {
    #[serde(default)]
    pub adapter: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(default)]
    pub emergency: Option<bool>,
    #[serde(default)]
    pub config: Option<serde_json::Value>,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub route: Option<serde_json::Value>,
}

/// Response for credential info
#[derive(Debug, Serialize)]
pub struct CredentialResponse {
    pub id: String,
    pub adapter: String,
    pub active: bool,
    pub emergency: bool,
    pub config: Option<serde_json::Value>,
    pub route: serde_json::Value,
    pub instance_status: Option<String>,
}

/// GET /admin/credentials/:id
pub async fn get_credential(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let config = state.config.read().await;

    let cred = config
        .credentials
        .get(&id)
        .ok_or_else(|| AppError::CredentialNotFound(id.clone()))?;

    let instance_status = state.manager.registry.get_status(&id).await;

    Ok(Json(CredentialResponse {
        id: id.clone(),
        adapter: cred.adapter.clone(),
        active: cred.active,
        emergency: cred.emergency,
        config: cred.config.clone(),
        route: cred.route.clone(),
        instance_status: instance_status.map(|s| format!("{:?}", s)),
    }))
}

/// POST /admin/credentials
pub async fn create_credential(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateCredentialRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Tell watcher to skip reloads
    set_skip_reload(&state).await;

    // Check if credential already exists
    {
        let config = state.config.read().await;
        if config.credentials.contains_key(&req.id) {
            return Err(AppError::Internal(format!(
                "Credential already exists: {}",
                req.id
            )));
        }
    }

    let cred_config = CredentialConfig {
        adapter: req.adapter,
        token: req.token,
        active: req.active,
        emergency: req.emergency,
        config: req.config,
        backend: req.backend,
        route: req.route,
    };

    // Update config in memory
    {
        let mut config = state.config.write().await;
        config
            .credentials
            .insert(req.id.clone(), cred_config.clone());
    }

    // Write config to file
    write_config(&state).await?;

    // Start task if active
    if req.active {
        state.manager.spawn_task(req.id.clone(), cred_config).await;
    }

    tracing::info!(credential_id = %req.id, "Credential created");

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": req.id,
            "status": "created"
        })),
    ))
}

/// PUT /admin/credentials/:id
pub async fn update_credential(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateCredentialRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Tell watcher to skip reloads
    set_skip_reload(&state).await;

    let old_config;
    let new_cred_config;

    // Update config in memory
    {
        let mut config = state.config.write().await;

        let cred = config
            .credentials
            .get_mut(&id)
            .ok_or_else(|| AppError::CredentialNotFound(id.clone()))?;

        old_config = cred.clone();

        // Apply updates
        if let Some(adapter) = req.adapter {
            cred.adapter = adapter;
        }
        if let Some(token) = req.token {
            cred.token = token;
        }
        if let Some(active) = req.active {
            cred.active = active;
        }
        if let Some(emergency) = req.emergency {
            cred.emergency = emergency;
        }
        if req.config.is_some() {
            cred.config = req.config;
        }
        if req.backend.is_some() {
            cred.backend = req.backend;
        }
        if let Some(route) = req.route {
            cred.route = route;
        }

        new_cred_config = cred.clone();
    }

    // Write config to file
    write_config(&state).await?;

    // Handle adapter instance lifecycle based on changes
    let needs_restart = old_config.adapter != new_cred_config.adapter
        || old_config.token != new_cred_config.token
        || old_config.config != new_cred_config.config;

    if needs_restart && old_config.active {
        state.manager.stop_task(&id).await;
    }

    if new_cred_config.active && (needs_restart || !old_config.active) {
        state.manager.spawn_task(id.clone(), new_cred_config).await;
    } else if !new_cred_config.active && old_config.active {
        state.manager.stop_task(&id).await;
    }

    tracing::info!(credential_id = %id, "Credential updated");

    Ok(Json(json!({
        "id": id,
        "status": "updated"
    })))
}

/// DELETE /admin/credentials/:id
pub async fn delete_credential(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    // Tell watcher to skip reloads
    set_skip_reload(&state).await;

    // Stop task if running
    state.manager.stop_task(&id).await;

    // Remove from config
    {
        let mut config = state.config.write().await;
        if config.credentials.remove(&id).is_none() {
            return Err(AppError::CredentialNotFound(id));
        }
    }

    // Write config to file
    write_config(&state).await?;

    tracing::info!(credential_id = %id, "Credential deleted");

    Ok(Json(json!({
        "id": id,
        "status": "deleted"
    })))
}

/// PATCH /admin/credentials/:id/activate
pub async fn activate_credential(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    // Tell watcher to skip reloads
    set_skip_reload(&state).await;

    let cred_config;

    {
        let mut config = state.config.write().await;

        let cred = config
            .credentials
            .get_mut(&id)
            .ok_or_else(|| AppError::CredentialNotFound(id.clone()))?;

        if cred.active {
            return Ok(Json(json!({
                "id": id,
                "status": "already_active"
            })));
        }

        cred.active = true;
        cred_config = cred.clone();
    }

    // Write config to file
    write_config(&state).await?;

    // Start task
    state.manager.spawn_task(id.clone(), cred_config).await;

    tracing::info!(credential_id = %id, "Credential activated");

    Ok(Json(json!({
        "id": id,
        "status": "activated"
    })))
}

/// PATCH /admin/credentials/:id/deactivate
pub async fn deactivate_credential(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    // Tell watcher to skip reloads
    set_skip_reload(&state).await;

    {
        let mut config = state.config.write().await;

        let cred = config
            .credentials
            .get_mut(&id)
            .ok_or_else(|| AppError::CredentialNotFound(id.clone()))?;

        if !cred.active {
            return Ok(Json(json!({
                "id": id,
                "status": "already_inactive"
            })));
        }

        cred.active = false;
    }

    // Write config to file
    write_config(&state).await?;

    // Stop task
    state.manager.stop_task(&id).await;

    tracing::info!(credential_id = %id, "Credential deactivated");

    Ok(Json(json!({
        "id": id,
        "status": "deactivated"
    })))
}

/// Set skip reload flag before any config modification
pub async fn set_skip_reload(state: &AppState) {
    use std::time::{Duration, Instant};
    let mut skip_until = state.skip_reload_until.write().await;
    *skip_until = Some(Instant::now() + Duration::from_secs(2));
}

/// Write config to file atomically (write to temp, then rename)
async fn write_config(state: &AppState) -> Result<(), AppError> {
    let config_path = std::env::var("GATEWAY_CONFIG").unwrap_or_else(|_| "config.json".to_string());

    let config = state.config.read().await;

    // Serialize config
    let json = serde_json::to_string_pretty(&*config)
        .map_err(|e| AppError::Internal(format!("Failed to serialize config: {}", e)))?;

    drop(config); // Release lock before file I/O

    // Write to temp file
    let temp_path = format!("{}.tmp", config_path);
    tokio::fs::write(&temp_path, &json)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write temp config: {}", e)))?;

    // Atomic rename
    tokio::fs::rename(&temp_path, &config_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to rename config: {}", e)))?;

    tracing::debug!("Config written to {}", config_path);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // ==================== Request/Response Struct Tests ====================

    #[test]
    fn test_create_credential_request_parse_minimal() {
        let json = r#"{
            "id": "test_cred",
            "adapter": "telegram",
            "token": "secret123",
            "route": {"type": "default"}
        }"#;

        let req: CreateCredentialRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.id, "test_cred");
        assert_eq!(req.adapter, "telegram");
        assert_eq!(req.token, "secret123");
        assert!(req.active); // default is true
        assert!(!req.emergency); // default is false
        assert!(req.config.is_none());
        assert!(req.backend.is_none());
    }

    #[test]
    fn test_create_credential_request_parse_full() {
        let json = r#"{
            "id": "test_cred",
            "adapter": "telegram",
            "token": "secret123",
            "active": false,
            "emergency": true,
            "config": {"chat_id": "123"},
            "backend": "pipelit",
            "route": {"type": "custom", "path": "/api"}
        }"#;

        let req: CreateCredentialRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.id, "test_cred");
        assert!(!req.active);
        assert!(req.emergency);
        assert!(req.config.is_some());
        assert_eq!(req.backend, Some("pipelit".to_string()));
    }

    #[test]
    fn test_update_credential_request_parse_empty() {
        let json = r#"{}"#;

        let req: UpdateCredentialRequest = serde_json::from_str(json).unwrap();
        assert!(req.adapter.is_none());
        assert!(req.token.is_none());
        assert!(req.active.is_none());
        assert!(req.emergency.is_none());
        assert!(req.config.is_none());
        assert!(req.backend.is_none());
        assert!(req.route.is_none());
    }

    #[test]
    fn test_update_credential_request_parse_partial() {
        let json = r#"{
            "active": true,
            "token": "new_token"
        }"#;

        let req: UpdateCredentialRequest = serde_json::from_str(json).unwrap();
        assert!(req.adapter.is_none());
        assert_eq!(req.token, Some("new_token".to_string()));
        assert_eq!(req.active, Some(true));
        assert!(req.emergency.is_none());
    }

    #[test]
    fn test_update_credential_request_parse_full() {
        let json = r#"{
            "adapter": "discord",
            "token": "new_token",
            "active": false,
            "emergency": true,
            "config": {"setting": "value"},
            "backend": "opencode",
            "route": {"new": "route"}
        }"#;

        let req: UpdateCredentialRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.adapter, Some("discord".to_string()));
        assert_eq!(req.token, Some("new_token".to_string()));
        assert_eq!(req.active, Some(false));
        assert_eq!(req.emergency, Some(true));
        assert!(req.config.is_some());
        assert_eq!(req.backend, Some("opencode".to_string()));
        assert!(req.route.is_some());
    }

    #[test]
    fn test_credential_response_serialize() {
        let response = CredentialResponse {
            id: "cred1".to_string(),
            adapter: "telegram".to_string(),
            active: true,
            emergency: false,
            config: Some(serde_json::json!({"key": "value"})),
            route: serde_json::json!({"type": "default"}),
            instance_status: Some("Running".to_string()),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"id\":\"cred1\""));
        assert!(json.contains("\"adapter\":\"telegram\""));
        assert!(json.contains("\"active\":true"));
        assert!(json.contains("\"emergency\":false"));
        assert!(json.contains("\"instance_status\":\"Running\""));
    }

    #[test]
    fn test_credential_response_serialize_minimal() {
        let response = CredentialResponse {
            id: "cred2".to_string(),
            adapter: "generic".to_string(),
            active: false,
            emergency: true,
            config: None,
            route: serde_json::json!(null),
            instance_status: None,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"id\":\"cred2\""));
        assert!(json.contains("\"config\":null"));
        assert!(json.contains("\"instance_status\":null"));
    }

    #[test]
    fn test_default_true() {
        assert!(default_true());
    }

    // ==================== Debug Trait Tests ====================

    #[test]
    fn test_create_credential_request_debug() {
        let req = CreateCredentialRequest {
            id: "test".to_string(),
            adapter: "telegram".to_string(),
            token: "secret".to_string(),
            active: true,
            emergency: false,
            config: None,
            backend: None,
            route: serde_json::json!({}),
        };

        let debug_str = format!("{:?}", req);
        assert!(debug_str.contains("CreateCredentialRequest"));
        assert!(debug_str.contains("test"));
    }

    #[test]
    fn test_update_credential_request_debug() {
        let req = UpdateCredentialRequest {
            adapter: Some("discord".to_string()),
            token: None,
            active: Some(true),
            emergency: None,
            config: None,
            backend: None,
            route: None,
        };

        let debug_str = format!("{:?}", req);
        assert!(debug_str.contains("UpdateCredentialRequest"));
        assert!(debug_str.contains("discord"));
    }

    #[test]
    fn test_credential_response_debug() {
        let response = CredentialResponse {
            id: "cred1".to_string(),
            adapter: "telegram".to_string(),
            active: true,
            emergency: false,
            config: None,
            route: serde_json::json!({}),
            instance_status: None,
        };

        let debug_str = format!("{:?}", response);
        assert!(debug_str.contains("CredentialResponse"));
        assert!(debug_str.contains("cred1"));
    }

    // ==================== Backend Name in Requests ====================

    #[test]
    fn test_create_request_with_backend_name() {
        let json = r#"{
            "id": "test",
            "adapter": "telegram",
            "token": "tok",
            "route": {},
            "backend": "pipelit"
        }"#;

        let req: CreateCredentialRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.backend, Some("pipelit".to_string()));
    }

    #[test]
    fn test_create_request_without_backend() {
        let json = r#"{
            "id": "test",
            "adapter": "generic",
            "token": "tok",
            "route": {}
        }"#;

        let req: CreateCredentialRequest = serde_json::from_str(json).unwrap();
        assert!(req.backend.is_none());
    }

    // ==================== Helper Function Tests ====================

    #[tokio::test]
    async fn test_set_skip_reload() {
        use std::time::Instant;
        use tokio::sync::RwLock;

        // Create minimal AppState-like structure for testing
        let skip_reload_until: RwLock<Option<Instant>> = RwLock::new(None);

        // Initially should be None
        assert!(skip_reload_until.read().await.is_none());

        // Simulate set_skip_reload behavior
        {
            use std::time::Duration;
            let mut skip_until = skip_reload_until.write().await;
            *skip_until = Some(Instant::now() + Duration::from_secs(2));
        }

        // Should now be Some
        let value = skip_reload_until.read().await;
        assert!(value.is_some());
        // Should be in the future
        assert!(value.unwrap() > Instant::now());
    }

    // ==================== Config Validation Tests ====================

    #[test]
    fn test_credential_config_from_create_request() {
        let req = CreateCredentialRequest {
            id: "test_id".to_string(),
            adapter: "telegram".to_string(),
            token: "test_token".to_string(),
            active: true,
            emergency: true,
            config: Some(serde_json::json!({"key": "value"})),
            backend: Some("pipelit".to_string()),
            route: serde_json::json!({"route": "data"}),
        };

        let cred_config = CredentialConfig {
            adapter: req.adapter.clone(),
            token: req.token.clone(),
            active: req.active,
            emergency: req.emergency,
            config: req.config.clone(),
            backend: req.backend.clone(),
            route: req.route.clone(),
        };

        assert_eq!(cred_config.adapter, "telegram");
        assert_eq!(cred_config.token, "test_token");
        assert!(cred_config.active);
        assert!(cred_config.emergency);
        assert!(cred_config.config.is_some());
        assert_eq!(cred_config.backend, Some("pipelit".to_string()));
    }

    #[test]
    fn test_update_applies_partial_changes() {
        let mut cred = CredentialConfig {
            adapter: "telegram".to_string(),
            token: "old_token".to_string(),
            active: true,
            emergency: false,
            config: None,
            backend: None,
            route: serde_json::json!({"old": "route"}),
        };

        let update = UpdateCredentialRequest {
            adapter: None,
            token: Some("new_token".to_string()),
            active: Some(false),
            emergency: None,
            config: None,
            backend: None,
            route: None,
        };

        // Apply updates (simulating update_credential logic)
        if let Some(token) = update.token {
            cred.token = token;
        }
        if let Some(active) = update.active {
            cred.active = active;
        }

        assert_eq!(cred.adapter, "telegram"); // unchanged
        assert_eq!(cred.token, "new_token"); // changed
        assert!(!cred.active); // changed
        assert!(!cred.emergency); // unchanged
    }
}
