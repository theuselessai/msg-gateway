use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Authentication failed")]
    Unauthorized,

    #[error("Credential not found: {0}")]
    CredentialNotFound(String),

    #[error("Credential inactive: {0}")]
    CredentialInactive(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Gone: {0}")]
    Gone(String),

    #[error("Payload too large: {0}")]
    PayloadTooLarge(String),

    #[error("Unsupported media type: {0}")]
    UnsupportedMediaType(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Forbidden: {0}")]
    #[allow(dead_code)]
    Forbidden(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::Config(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized".to_string()),
            AppError::CredentialNotFound(id) => (
                StatusCode::NOT_FOUND,
                format!("Credential not found: {}", id),
            ),
            AppError::CredentialInactive(id) => (
                StatusCode::BAD_REQUEST,
                format!("Credential inactive: {}", id),
            ),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::Gone(msg) => (StatusCode::GONE, msg.clone()),
            AppError::PayloadTooLarge(msg) => (StatusCode::PAYLOAD_TOO_LARGE, msg.clone()),
            AppError::UnsupportedMediaType(msg) => {
                (StatusCode::UNSUPPORTED_MEDIA_TYPE, msg.clone())
            }
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            AppError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
        };

        let body = Json(json!({
            "error": message
        }));

        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::StatusCode;

    #[test]
    fn test_app_error_display() {
        assert_eq!(
            AppError::Config("bad config".to_string()).to_string(),
            "Configuration error: bad config"
        );
        assert_eq!(AppError::Unauthorized.to_string(), "Authentication failed");
        assert_eq!(
            AppError::CredentialNotFound("cred1".to_string()).to_string(),
            "Credential not found: cred1"
        );
        assert_eq!(
            AppError::CredentialInactive("cred2".to_string()).to_string(),
            "Credential inactive: cred2"
        );
        assert_eq!(
            AppError::NotFound("resource".to_string()).to_string(),
            "Not found: resource"
        );
        assert_eq!(
            AppError::Gone("expired".to_string()).to_string(),
            "Gone: expired"
        );
        assert_eq!(
            AppError::PayloadTooLarge("too big".to_string()).to_string(),
            "Payload too large: too big"
        );
        assert_eq!(
            AppError::UnsupportedMediaType("bad type".to_string()).to_string(),
            "Unsupported media type: bad type"
        );
        assert_eq!(
            AppError::BadRequest("missing field".to_string()).to_string(),
            "Bad request: missing field"
        );
        assert_eq!(
            AppError::Internal("oops".to_string()).to_string(),
            "Internal error: oops"
        );
    }

    #[test]
    fn test_app_error_debug() {
        let err = AppError::Config("test".to_string());
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("Config"));
        assert!(debug_str.contains("test"));
    }

    #[tokio::test]
    async fn test_app_error_into_response_config() {
        let err = AppError::Config("invalid config".to_string());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "invalid config");
    }

    #[tokio::test]
    async fn test_app_error_into_response_unauthorized() {
        let err = AppError::Unauthorized;
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "Unauthorized");
    }

    #[tokio::test]
    async fn test_app_error_into_response_credential_not_found() {
        let err = AppError::CredentialNotFound("test_cred".to_string());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "Credential not found: test_cred");
    }

    #[tokio::test]
    async fn test_app_error_into_response_credential_inactive() {
        let err = AppError::CredentialInactive("inactive_cred".to_string());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "Credential inactive: inactive_cred");
    }

    #[tokio::test]
    async fn test_app_error_into_response_not_found() {
        let err = AppError::NotFound("resource missing".to_string());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "resource missing");
    }

    #[tokio::test]
    async fn test_app_error_into_response_gone() {
        let err = AppError::Gone("file expired".to_string());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::GONE);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "file expired");
    }

    #[tokio::test]
    async fn test_app_error_into_response_payload_too_large() {
        let err = AppError::PayloadTooLarge("File too large".to_string());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "File too large");
    }

    #[tokio::test]
    async fn test_app_error_into_response_unsupported_media_type() {
        let err = AppError::UnsupportedMediaType("MIME type not allowed".to_string());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "MIME type not allowed");
    }

    #[tokio::test]
    async fn test_app_error_into_response_bad_request() {
        let err = AppError::BadRequest("Missing required field".to_string());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "Missing required field");
    }

    #[tokio::test]
    async fn test_app_error_into_response_internal() {
        let err = AppError::Internal("something broke".to_string());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "something broke");
    }

    #[tokio::test]
    async fn test_forbidden_error() {
        let err = AppError::Forbidden("access denied".to_string());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "access denied");
    }
}
