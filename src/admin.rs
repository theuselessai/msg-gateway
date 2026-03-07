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

use crate::config::{CredentialConfig, TargetConfig};
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
    /// Adapter-specific configuration
    #[serde(default)]
    pub config: Option<serde_json::Value>,
    /// Per-credential backend target override
    #[serde(default)]
    pub target: Option<TargetConfig>,
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
    /// Adapter-specific configuration
    #[serde(default)]
    pub config: Option<serde_json::Value>,
    /// Per-credential backend target override
    #[serde(default)]
    pub target: Option<TargetConfig>,
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
        target: req.target,
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
        if req.target.is_some() {
            cred.target = req.target;
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
