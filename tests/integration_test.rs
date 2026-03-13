//! Integration tests for plit-gw
//!
//! These tests spawn a real gateway server and mock backend to test the full flow.

use plit_gw::adapter::AdapterInstanceManager;
use plit_gw::backend::ExternalBackendManager;
use plit_gw::config::{
    AuthConfig, BackendConfig, BackendProtocol, Config, CredentialConfig, FileCacheConfig,
    GatewayConfig,
};
use plit_gw::manager::CredentialManager;
use serial_test::serial;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Create a minimal test config
fn test_config(port: u16) -> Config {
    let mut backends = HashMap::new();
    backends.insert(
        "default".to_string(),
        BackendConfig {
            protocol: BackendProtocol::Pipelit,
            inbound_url: Some("http://127.0.0.1:18000/inbound".to_string()),
            base_url: None,
            token: "test_backend_token".to_string(),
            poll_interval_ms: None,
            adapter_dir: None,
            port: None,
            active: true,
            config: None,
        },
    );
    Config {
        gateway: GatewayConfig {
            listen: format!("127.0.0.1:{}", port),
            admin_token: "test_admin_token".to_string(),
            default_backend: Some("default".to_string()),
            adapters_dir: "./adapters".to_string(),
            adapter_port_range: (19000, 19100),
            backends_dir: "./backends".to_string(),
            backend_port_range: (19200, 19300),
            file_cache: None,
            guardrails_dir: None,
        },
        auth: AuthConfig {
            send_token: "test_send_token".to_string(),
        },
        health_checks: HashMap::new(),
        backends,
        credentials: {
            let mut creds = HashMap::new();
            creds.insert(
                "test_generic".to_string(),
                CredentialConfig {
                    adapter: "generic".to_string(),
                    token: "generic_token".to_string(),
                    active: true,
                    emergency: false,
                    route: serde_json::json!({"channel": "test"}),
                    config: None,
                    backend: None,
                },
            );
            creds
        },
    }
}

/// Create test config with file cache enabled
#[allow(dead_code)]
fn test_config_with_file_cache(port: u16, cache_dir: &str) -> Config {
    let mut config = test_config(port);
    config.gateway.file_cache = Some(FileCacheConfig {
        directory: cache_dir.to_string(),
        max_file_size_mb: 10,
        max_cache_size_mb: 100,
        ttl_hours: 24,
        cleanup_interval_minutes: 60,
        allowed_mime_types: vec!["*/*".to_string()],
        blocked_mime_types: vec![],
    });
    config
}

/// Helper to find an available port
async fn find_available_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

/// Test server handle for cleanup
struct TestServer {
    handle: tokio::task::JoinHandle<()>,
    #[allow(dead_code)]
    state: Arc<plit_gw::server::AppState>,
    port: u16,
    admin_token: String,
    send_token: String,
}

impl TestServer {
    async fn new(config: Config) -> Self {
        let port = config
            .gateway
            .listen
            .split(':')
            .next_back()
            .unwrap()
            .parse()
            .unwrap();
        let admin_token = config.gateway.admin_token.clone();
        let send_token = config.auth.send_token.clone();

        let manager = Arc::new(CredentialManager::new());
        let adapter_manager = Arc::new(
            AdapterInstanceManager::new(
                config.gateway.adapters_dir.clone(),
                config.gateway.adapter_port_range,
                &config.gateway.listen,
            )
            .unwrap(),
        );

        let backend_manager = Arc::new(ExternalBackendManager::new(
            config.gateway.backends_dir.clone(),
            config.gateway.backend_port_range,
            &config.gateway.listen,
            config.auth.send_token.clone(),
        ));

        let (state, server_future) =
            plit_gw::server::create_server(config, manager, adapter_manager, backend_manager)
                .await
                .unwrap();

        let handle = tokio::spawn(async move {
            let _ = server_future.await;
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        Self {
            handle,
            state,
            port,
            admin_token,
            send_token,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }

    fn client(&self) -> reqwest::Client {
        reqwest::Client::new()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

// ============================================================================
// Health Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_health_endpoint() {
    let port = find_available_port().await;
    let server = TestServer::new(test_config(port)).await;

    let resp = server
        .client()
        .get(server.url("/health"))
        .send()
        .await
        .unwrap();

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
}

// ============================================================================
// Auth Tests
// ============================================================================

#[tokio::test]
async fn test_admin_auth() {
    let port = find_available_port().await;
    let server = TestServer::new(test_config(port)).await;
    let client = server.client();

    // Test without auth - should fail
    let resp = client
        .get(server.url("/admin/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Test with wrong token - should fail
    let resp = client
        .get(server.url("/admin/health"))
        .header("Authorization", "Bearer wrong_token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Test with correct token - should succeed
    let resp = client
        .get(server.url("/admin/health"))
        .header("Authorization", format!("Bearer {}", server.admin_token))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
}

#[tokio::test]
async fn test_send_auth() {
    let port = find_available_port().await;
    let server = TestServer::new(test_config(port)).await;
    let client = server.client();

    // Test without auth - should fail
    let resp = client
        .post(server.url("/api/v1/send"))
        .json(&serde_json::json!({
            "credential_id": "test_generic",
            "chat_id": "chat1",
            "text": "Hello"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Test with correct token - should succeed
    let resp = client
        .post(server.url("/api/v1/send"))
        .header("Authorization", format!("Bearer {}", server.send_token))
        .json(&serde_json::json!({
            "credential_id": "test_generic",
            "chat_id": "chat1",
            "text": "Hello"
        }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
}

// ============================================================================
// Credential CRUD Tests
// ============================================================================

#[tokio::test]
async fn test_list_credentials() {
    let port = find_available_port().await;
    let server = TestServer::new(test_config(port)).await;
    let client = server.client();

    let resp = client
        .get(server.url("/admin/credentials"))
        .header("Authorization", format!("Bearer {}", server.admin_token))
        .send()
        .await
        .unwrap();

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    let credentials = body["credentials"].as_array().unwrap();
    assert_eq!(credentials.len(), 1);
    assert_eq!(credentials[0]["id"], "test_generic");
    assert_eq!(credentials[0]["adapter"], "generic");
    assert_eq!(credentials[0]["active"], true);
}

#[tokio::test]
async fn test_get_credential() {
    let port = find_available_port().await;
    let server = TestServer::new(test_config(port)).await;
    let client = server.client();

    // Get existing credential
    let resp = client
        .get(server.url("/admin/credentials/test_generic"))
        .header("Authorization", format!("Bearer {}", server.admin_token))
        .send()
        .await
        .unwrap();

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["id"], "test_generic");
    assert_eq!(body["adapter"], "generic");
    assert_eq!(body["active"], true);

    // Get non-existent credential
    let resp = client
        .get(server.url("/admin/credentials/nonexistent"))
        .header("Authorization", format!("Bearer {}", server.admin_token))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);
}

// ============================================================================
// File Upload API Tests
// ============================================================================

#[tokio::test]
async fn test_file_upload_and_download() {
    let port = find_available_port().await;
    let temp_dir = tempfile::TempDir::new().unwrap();
    let server = TestServer::new(test_config_with_file_cache(
        port,
        &temp_dir.path().to_string_lossy(),
    ))
    .await;
    let client = server.client();

    // Upload file via multipart
    let file_content = b"Hello, this is test file content!";
    let file_part = reqwest::multipart::Part::bytes(file_content.to_vec())
        .file_name("test_document.txt")
        .mime_str("text/plain")
        .unwrap();

    let form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("filename", "test_document.txt")
        .text("mime_type", "text/plain");

    let resp = client
        .post(server.url("/api/v1/files"))
        .header("Authorization", format!("Bearer {}", server.send_token))
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["file_id"].as_str().unwrap().starts_with("f_"));
    assert_eq!(body["filename"], "test_document.txt");
    assert_eq!(body["mime_type"], "text/plain");
    assert_eq!(body["size_bytes"], file_content.len() as u64);
    assert!(body["download_url"].as_str().unwrap().contains("/files/"));

    // Download file via GET /files/{file_id} (no auth required)
    let file_id = body["file_id"].as_str().unwrap();
    let resp = client
        .get(server.url(&format!("/files/{}", file_id)))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap(),
        "text/plain"
    );
    let downloaded = resp.bytes().await.unwrap();
    assert_eq!(downloaded.as_ref(), file_content);
}

#[tokio::test]
async fn test_file_upload_no_auth() {
    let port = find_available_port().await;
    let temp_dir = tempfile::TempDir::new().unwrap();
    let server = TestServer::new(test_config_with_file_cache(
        port,
        &temp_dir.path().to_string_lossy(),
    ))
    .await;
    let client = server.client();

    let file_part = reqwest::multipart::Part::bytes(b"data".to_vec())
        .file_name("test.txt")
        .mime_str("text/plain")
        .unwrap();

    let form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("filename", "test.txt");

    // No Authorization header
    let resp = client
        .post(server.url("/api/v1/files"))
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn test_file_upload_wrong_auth() {
    let port = find_available_port().await;
    let temp_dir = tempfile::TempDir::new().unwrap();
    let server = TestServer::new(test_config_with_file_cache(
        port,
        &temp_dir.path().to_string_lossy(),
    ))
    .await;
    let client = server.client();

    let file_part = reqwest::multipart::Part::bytes(b"data".to_vec())
        .file_name("test.txt")
        .mime_str("text/plain")
        .unwrap();

    let form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("filename", "test.txt");

    let resp = client
        .post(server.url("/api/v1/files"))
        .header("Authorization", "Bearer wrong_token")
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn test_file_upload_filename_from_multipart() {
    let port = find_available_port().await;
    let temp_dir = tempfile::TempDir::new().unwrap();
    let server = TestServer::new(test_config_with_file_cache(
        port,
        &temp_dir.path().to_string_lossy(),
    ))
    .await;
    let client = server.client();

    // Upload with filename only in multipart Content-Disposition (no explicit filename field)
    let file_part = reqwest::multipart::Part::bytes(b"content".to_vec())
        .file_name("from_multipart.txt")
        .mime_str("text/plain")
        .unwrap();

    let form = reqwest::multipart::Form::new().part("file", file_part);

    let resp = client
        .post(server.url("/api/v1/files"))
        .header("Authorization", format!("Bearer {}", server.send_token))
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["filename"], "from_multipart.txt");
}

#[tokio::test]
async fn test_file_upload_default_mime_type() {
    let port = find_available_port().await;
    let temp_dir = tempfile::TempDir::new().unwrap();
    let server = TestServer::new(test_config_with_file_cache(
        port,
        &temp_dir.path().to_string_lossy(),
    ))
    .await;
    let client = server.client();

    // Upload without specifying mime_type — should default to application/octet-stream
    let file_part = reqwest::multipart::Part::bytes(b"binary data".to_vec())
        .file_name("data.bin")
        .mime_str("application/octet-stream")
        .unwrap();

    let form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("filename", "data.bin");

    let resp = client
        .post(server.url("/api/v1/files"))
        .header("Authorization", format!("Bearer {}", server.send_token))
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["mime_type"], "application/octet-stream");
}

#[tokio::test]
async fn test_file_download_not_found() {
    let port = find_available_port().await;
    let temp_dir = tempfile::TempDir::new().unwrap();
    let server = TestServer::new(test_config_with_file_cache(
        port,
        &temp_dir.path().to_string_lossy(),
    ))
    .await;
    let client = server.client();

    // Try to download non-existent file (no auth needed)
    let resp = client
        .get(server.url("/files/f_nonexistent"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn test_file_upload_no_file_cache() {
    let port = find_available_port().await;
    // Use config without file cache
    let server = TestServer::new(test_config(port)).await;
    let client = server.client();

    let file_part = reqwest::multipart::Part::bytes(b"data".to_vec())
        .file_name("test.txt")
        .mime_str("text/plain")
        .unwrap();

    let form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("filename", "test.txt");

    let resp = client
        .post(server.url("/api/v1/files"))
        .header("Authorization", format!("Bearer {}", server.send_token))
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 500);
}

#[tokio::test]
#[serial]
async fn test_update_credential() {
    let port = find_available_port().await;

    // Create temp config file
    let temp_dir = std::env::temp_dir();
    let config_path = temp_dir.join(format!("test_config_update_{}.json", port));
    let config = test_config(port);
    std::fs::write(&config_path, serde_json::to_string(&config).unwrap()).unwrap();
    unsafe {
        std::env::set_var("GATEWAY_CONFIG", &config_path);
    }

    let server = TestServer::new(config).await;
    let client = server.client();

    // Update existing credential
    let resp = client
        .put(server.url("/admin/credentials/test_generic"))
        .header("Authorization", format!("Bearer {}", server.admin_token))
        .json(&serde_json::json!({
            "emergency": true,
            "route": {"channel": "updated"}
        }))
        .send()
        .await
        .unwrap();

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "updated");

    // Verify update
    let resp = client
        .get(server.url("/admin/credentials/test_generic"))
        .header("Authorization", format!("Bearer {}", server.admin_token))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["emergency"], true);
    assert_eq!(body["route"]["channel"], "updated");

    // Update non-existent credential
    let resp = client
        .put(server.url("/admin/credentials/nonexistent"))
        .header("Authorization", format!("Bearer {}", server.admin_token))
        .json(&serde_json::json!({"active": false}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);

    // Cleanup
    let _ = std::fs::remove_file(&config_path);
}

#[tokio::test]
#[serial]
async fn test_delete_credential() {
    let port = find_available_port().await;

    // Create temp config file for write_config to work
    let temp_dir = std::env::temp_dir();
    let config_path = temp_dir.join(format!("test_config_{}.json", port));
    let config = test_config(port);
    std::fs::write(&config_path, serde_json::to_string(&config).unwrap()).unwrap();

    // Set env var before creating server
    unsafe {
        std::env::set_var("GATEWAY_CONFIG", &config_path);
    }

    let server = TestServer::new(config).await;
    let client = server.client();

    // Delete existing credential
    let resp = client
        .delete(server.url("/admin/credentials/test_generic"))
        .header("Authorization", format!("Bearer {}", server.admin_token))
        .send()
        .await
        .unwrap();

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "deleted");

    // Verify it's gone
    let resp = client
        .get(server.url("/admin/credentials/test_generic"))
        .header("Authorization", format!("Bearer {}", server.admin_token))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);

    // Delete non-existent credential
    let resp = client
        .delete(server.url("/admin/credentials/nonexistent"))
        .header("Authorization", format!("Bearer {}", server.admin_token))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);

    // Cleanup
    let _ = std::fs::remove_file(&config_path);
}

#[tokio::test]
#[serial]
async fn test_activate_deactivate_credential() {
    let port = find_available_port().await;

    // Create temp config file for write_config to work
    let temp_dir = std::env::temp_dir();
    let config_path = temp_dir.join(format!("test_config_actdeact_{}.json", port));
    let config = test_config(port);
    std::fs::write(&config_path, serde_json::to_string(&config).unwrap()).unwrap();

    // Set env var before creating server
    unsafe {
        std::env::set_var("GATEWAY_CONFIG", &config_path);
    }

    let server = TestServer::new(config).await;
    let client = server.client();

    // Deactivate
    let resp = client
        .patch(server.url("/admin/credentials/test_generic/deactivate"))
        .header("Authorization", format!("Bearer {}", server.admin_token))
        .send()
        .await
        .unwrap();

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "deactivated");

    // Verify deactivated
    let resp = client
        .get(server.url("/admin/credentials/test_generic"))
        .header("Authorization", format!("Bearer {}", server.admin_token))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["active"], false);

    // Deactivate again - should say already inactive
    let resp = client
        .patch(server.url("/admin/credentials/test_generic/deactivate"))
        .header("Authorization", format!("Bearer {}", server.admin_token))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "already_inactive");

    // Activate
    let resp = client
        .patch(server.url("/admin/credentials/test_generic/activate"))
        .header("Authorization", format!("Bearer {}", server.admin_token))
        .send()
        .await
        .unwrap();

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "activated");

    // Activate again - should say already active
    let resp = client
        .patch(server.url("/admin/credentials/test_generic/activate"))
        .header("Authorization", format!("Bearer {}", server.admin_token))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "already_active");

    // Activate non-existent
    let resp = client
        .patch(server.url("/admin/credentials/nonexistent/activate"))
        .header("Authorization", format!("Bearer {}", server.admin_token))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);

    // Cleanup
    let _ = std::fs::remove_file(&config_path);
}

// ============================================================================
// Send Message Tests
// ============================================================================

#[tokio::test]
async fn test_send_to_nonexistent_credential() {
    let port = find_available_port().await;
    let server = TestServer::new(test_config(port)).await;
    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/send"))
        .header("Authorization", format!("Bearer {}", server.send_token))
        .json(&serde_json::json!({
            "credential_id": "nonexistent",
            "chat_id": "chat1",
            "text": "Hello"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn test_send_to_inactive_credential() {
    let port = find_available_port().await;
    let mut config = test_config(port);
    config.credentials.get_mut("test_generic").unwrap().active = false;

    let server = TestServer::new(config).await;
    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/send"))
        .header("Authorization", format!("Bearer {}", server.send_token))
        .json(&serde_json::json!({
            "credential_id": "test_generic",
            "chat_id": "chat1",
            "text": "Hello"
        }))
        .send()
        .await
        .unwrap();

    // Should fail because credential is inactive
    assert!(!resp.status().is_success());
}

#[tokio::test]
async fn test_send_missing_fields() {
    let port = find_available_port().await;
    let server = TestServer::new(test_config(port)).await;
    let client = server.client();

    // Missing credential_id
    let resp = client
        .post(server.url("/api/v1/send"))
        .header("Authorization", format!("Bearer {}", server.send_token))
        .json(&serde_json::json!({
            "chat_id": "chat1",
            "text": "Hello"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 500);

    // Missing chat_id
    let resp = client
        .post(server.url("/api/v1/send"))
        .header("Authorization", format!("Bearer {}", server.send_token))
        .json(&serde_json::json!({
            "credential_id": "test_generic",
            "text": "Hello"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 500);
}

// ============================================================================
// Generic Adapter REST Tests
// ============================================================================

#[tokio::test]
async fn test_generic_chat_endpoint() {
    let port = find_available_port().await;
    let server = TestServer::new(test_config(port)).await;
    let client = server.client();

    // Send message via generic chat endpoint
    let resp = client
        .post(server.url("/api/v1/chat/test_generic"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "Hello from generic",
            "chat_id": "chat123",
            "from": {
                "id": "user1",
                "display_name": "Test User"
            }
        }))
        .send()
        .await
        .unwrap();

    // Should return 202 Accepted (backend may not be reachable in test)
    // Or success if it went through
    assert!(resp.status().is_success() || resp.status() == 202);
}

#[tokio::test]
async fn test_generic_chat_wrong_token() {
    let port = find_available_port().await;
    let server = TestServer::new(test_config(port)).await;
    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/chat/test_generic"))
        .header("Authorization", "Bearer wrong_token")
        .json(&serde_json::json!({
            "text": "Hello",
            "chat_id": "chat123",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn test_generic_chat_nonexistent_credential() {
    let port = find_available_port().await;
    let server = TestServer::new(test_config(port)).await;
    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/chat/nonexistent"))
        .header("Authorization", "Bearer some_token")
        .json(&serde_json::json!({
            "text": "Hello",
            "chat_id": "chat123",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);
}

// ============================================================================
// Generic Adapter File Support Tests (PR #28)
// ============================================================================

/// Spawn a minimal HTTP file server that serves `content` at `path`
async fn spawn_file_server(
    path: &'static str,
    content: &'static str,
) -> (u16, tokio::task::JoinHandle<()>) {
    use axum::routing::get;
    let app = axum::Router::new().route(path, get(move || async move { content }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (port, handle)
}

/// Spawn a mock backend that captures one inbound POST body and returns 200.
/// Returns (port, receiver) — await the receiver to get the captured JSON body.
async fn spawn_mock_backend() -> (u16, tokio::sync::oneshot::Receiver<serde_json::Value>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = tokio::sync::oneshot::channel();

    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            // Read the full HTTP request
            let mut buf = vec![0u8; 65536];
            let mut total = 0;
            loop {
                match stream.read(&mut buf[total..]).await {
                    Ok(0) => break,
                    Ok(n) => {
                        total += n;
                        // Check if we have the full HTTP request (headers + body)
                        let so_far = &buf[..total];
                        if let Some(header_end) = find_header_end(so_far) {
                            // Parse Content-Length to know when body is complete
                            let headers_str =
                                std::str::from_utf8(&so_far[..header_end]).unwrap_or("");
                            let content_length = parse_content_length(headers_str);
                            let body_received = total - (header_end + 4);
                            if body_received >= content_length {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }

            // Extract JSON body (after \r\n\r\n)
            let raw = &buf[..total];
            let body_json = if let Some(header_end) = find_header_end(raw) {
                let body_bytes = &raw[header_end + 4..];
                serde_json::from_slice(body_bytes).unwrap_or(serde_json::Value::Null)
            } else {
                serde_json::Value::Null
            };

            // Send HTTP 200 response
            let response =
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Type: application/json\r\n\r\nok";
            let _ = stream.write_all(response).await;

            let _ = tx.send(body_json);
        }
    });

    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    (port, rx)
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn parse_content_length(headers: &str) -> usize {
    for line in headers.lines() {
        let lower = line.to_lowercase();
        if lower.starts_with("content-length:")
            && let Some(val) = lower.split(':').nth(1)
        {
            return val.trim().parse().unwrap_or(0);
        }
    }
    0
}

/// Create test config with a custom backend inbound URL
fn test_config_with_backend(gateway_port: u16, backend_port: u16) -> Config {
    let mut config = test_config(gateway_port);
    if let Some(b) = config.backends.get_mut("default") {
        b.inbound_url = Some(format!("http://127.0.0.1:{}/inbound", backend_port));
    }
    config
}

/// Create test config with file cache and a custom backend inbound URL
fn test_config_with_file_cache_and_backend(
    gateway_port: u16,
    cache_dir: &str,
    backend_port: u16,
) -> Config {
    let mut config = test_config_with_file_cache(gateway_port, cache_dir);
    if let Some(b) = config.backends.get_mut("default") {
        b.inbound_url = Some(format!("http://127.0.0.1:{}/inbound", backend_port));
    }
    config
}

/// Test 1: Generic inbound with file — file is downloaded and cached successfully.
/// The backend receives an InboundMessage with attachments[0].download_url starting with "http".
#[tokio::test]
async fn test_generic_inbound_with_file_success() {
    // Spin up a file server serving a small text file
    let (file_port, _file_server) = spawn_file_server("/file.txt", "hello").await;

    // Spin up mock backend to capture the forwarded inbound message
    let (backend_port, backend_rx) = spawn_mock_backend().await;

    let gateway_port = find_available_port().await;
    let temp_dir = tempfile::TempDir::new().unwrap();
    let server = TestServer::new(test_config_with_file_cache_and_backend(
        gateway_port,
        &temp_dir.path().to_string_lossy(),
        backend_port,
    ))
    .await;
    let client = server.client();

    // POST inbound message with a file reference
    let resp = client
        .post(server.url("/api/v1/chat/test_generic"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "chat_id": "c1",
            "text": "see file",
            "from": {"id": "u1"},
            "files": [{
                "url": format!("http://127.0.0.1:{}/file.txt", file_port),
                "filename": "file.txt",
                "mime_type": "text/plain"
            }]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 202);

    // Wait for the backend to receive the forwarded message (with timeout)
    let body = tokio::time::timeout(std::time::Duration::from_secs(5), backend_rx)
        .await
        .expect("Timed out waiting for backend")
        .expect("Backend receiver dropped");

    // Verify attachments were forwarded with a valid download URL
    let attachments = body["attachments"].as_array().expect("attachments array");
    assert_eq!(attachments.len(), 1, "Expected 1 attachment");
    let download_url = attachments[0]["download_url"]
        .as_str()
        .expect("download_url string");
    assert!(
        download_url.starts_with("http"),
        "Expected download_url to start with 'http', got: {}",
        download_url
    );
}

/// Test 2: Generic inbound with file — download fails (URL unreachable).
/// Gateway still returns 202 (non-fatal), backend receives message with NO attachments (failed ones are skipped).
#[tokio::test]
async fn test_generic_inbound_with_file_download_failure() {
    // Use a port with nothing listening to simulate a 404/connection refused
    let dead_port = find_available_port().await;
    // Don't bind it — just use the port number so connection is refused

    let (backend_port, backend_rx) = spawn_mock_backend().await;

    let gateway_port = find_available_port().await;
    let temp_dir = tempfile::TempDir::new().unwrap();
    let server = TestServer::new(test_config_with_file_cache_and_backend(
        gateway_port,
        &temp_dir.path().to_string_lossy(),
        backend_port,
    ))
    .await;
    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/chat/test_generic"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "chat_id": "c2",
            "text": "bad file",
            "from": {"id": "u2"},
            "files": [{
                "url": format!("http://127.0.0.1:{}/missing.txt", dead_port),
                "filename": "missing.txt",
                "mime_type": "text/plain"
            }]
        }))
        .send()
        .await
        .unwrap();

    // Should still return 202 — file errors are non-fatal
    assert_eq!(resp.status(), 202);

    // Backend should still receive the message but with NO attachments (failed ones are skipped)
    let body = tokio::time::timeout(std::time::Duration::from_secs(5), backend_rx)
        .await
        .expect("Timed out waiting for backend")
        .expect("Backend receiver dropped");

    let attachments = body
        .get("attachments")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    assert_eq!(
        attachments, 0,
        "Expected 0 attachments (failed file was skipped)"
    );
}

/// Test 3: Generic inbound with files but NO file cache configured.
/// Files are silently ignored (warn logged), backend receives message with empty attachments.
#[tokio::test]
async fn test_generic_inbound_with_file_no_cache() {
    let (backend_port, backend_rx) = spawn_mock_backend().await;

    let gateway_port = find_available_port().await;
    // Use config WITHOUT file cache
    let server = TestServer::new(test_config_with_backend(gateway_port, backend_port)).await;
    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/chat/test_generic"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "chat_id": "c3",
            "text": "file ignored",
            "from": {"id": "u3"},
            "files": [{
                "url": "http://127.0.0.1:1/ignored.txt",
                "filename": "ignored.txt",
                "mime_type": "text/plain"
            }]
        }))
        .send()
        .await
        .unwrap();

    // Should return 202 — files are silently ignored when no cache
    assert_eq!(resp.status(), 202);

    // Backend receives message with no attachments (files were ignored)
    let body = tokio::time::timeout(std::time::Duration::from_secs(5), backend_rx)
        .await
        .expect("Timed out waiting for backend")
        .expect("Backend receiver dropped");

    // attachments field is either absent or an empty array (skip_serializing_if = "Vec::is_empty")
    let attachments = body.get("attachments");
    let is_empty = attachments
        .map(|a| a.as_array().map(|arr| arr.is_empty()).unwrap_or(true))
        .unwrap_or(true);
    assert!(
        is_empty,
        "Expected empty or absent attachments, got: {:?}",
        attachments
    );
}

/// Test 4: Send to generic adapter with file_ids — WebSocket client receives file_urls.
#[tokio::test]
async fn test_send_to_generic_with_file_ids_includes_file_urls() {
    use futures_util::StreamExt;
    use tokio_tungstenite::connect_async;

    let gateway_port = find_available_port().await;
    let temp_dir = tempfile::TempDir::new().unwrap();
    let server = TestServer::new(test_config_with_file_cache(
        gateway_port,
        &temp_dir.path().to_string_lossy(),
    ))
    .await;
    let client = server.client();

    // Step 1: Upload a file to get a file_id and download_url
    let file_content = b"test file for ws";
    let file_part = reqwest::multipart::Part::bytes(file_content.to_vec())
        .file_name("ws_test.txt")
        .mime_str("text/plain")
        .unwrap();
    let form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("filename", "ws_test.txt")
        .text("mime_type", "text/plain");

    let upload_resp = client
        .post(server.url("/api/v1/files"))
        .header("Authorization", format!("Bearer {}", server.send_token))
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(upload_resp.status(), 200);
    let upload_body: serde_json::Value = upload_resp.json().await.unwrap();
    let file_id = upload_body["file_id"].as_str().unwrap().to_string();
    let expected_download_url = upload_body["download_url"].as_str().unwrap().to_string();

    // Step 2: Connect a WebSocket to the generic adapter chat
    let ws_url = format!("ws://127.0.0.1:{}/ws/chat/test_generic/chat1", server.port);
    let request = http::Request::builder()
        .uri(&ws_url)
        .header("Authorization", "Bearer generic_token")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("Sec-WebSocket-Version", "13")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Host", format!("127.0.0.1:{}", server.port))
        .body(())
        .unwrap();

    let (ws_stream, _) = connect_async(request)
        .await
        .expect("WebSocket connect failed");
    let (_write, mut read) = ws_stream.split();

    // Give the WS connection time to register in the registry
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Step 3: Send a message with file_ids via the send API
    let send_resp = client
        .post(server.url("/api/v1/send"))
        .header("Authorization", format!("Bearer {}", server.send_token))
        .json(&serde_json::json!({
            "credential_id": "test_generic",
            "chat_id": "chat1",
            "text": "here",
            "file_ids": [file_id]
        }))
        .send()
        .await
        .unwrap();

    assert!(
        send_resp.status().is_success(),
        "Send failed: {:?}",
        send_resp.status()
    );

    // Step 4: Assert WebSocket receives a message with file_urls containing the download URL
    let ws_msg = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(msg) = read.next().await {
            if let Ok(tokio_tungstenite::tungstenite::Message::Text(text)) = msg {
                return text.to_string();
            }
        }
        String::new()
    })
    .await
    .expect("Timed out waiting for WebSocket message");

    assert!(!ws_msg.is_empty(), "No WebSocket message received");

    let ws_body: serde_json::Value =
        serde_json::from_str(&ws_msg).expect("WS message is not valid JSON");

    let file_urls = ws_body["file_urls"]
        .as_array()
        .expect("file_urls should be an array");
    assert_eq!(file_urls.len(), 1, "Expected 1 file_url");
    assert_eq!(
        file_urls[0].as_str().unwrap(),
        expected_download_url,
        "file_url does not match expected download URL"
    );
}

// ============================================================================
// OpenCode Backend Integration Tests
// ============================================================================

/// Captured HTTP request from mock OpenCode server
#[derive(Debug, Clone)]
struct MockCapturedRequest {
    path: String,
    headers: Vec<(String, String)>,
    body: serde_json::Value,
}

/// Spawn a mock OpenCode HTTP server that implements:
/// - POST /session → `{"id": "mock-session-id"}`
/// - POST /session/{id}/message → AI response (or 500 if `error_on_message`)
///
/// Returns (port, captured_requests, join_handle).
async fn spawn_mock_opencode(
    error_on_message: bool,
) -> (
    u16,
    Arc<std::sync::Mutex<Vec<MockCapturedRequest>>>,
    tokio::task::JoinHandle<()>,
) {
    use axum::routing::post;

    let captured: Arc<std::sync::Mutex<Vec<MockCapturedRequest>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));

    let cap_session = captured.clone();
    let cap_message = captured.clone();

    let app = axum::Router::new()
        .route(
            "/session",
            post(move |headers: axum::http::HeaderMap, body: String| {
                let cap = cap_session.clone();
                async move {
                    let body_json: serde_json::Value =
                        serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
                    let hdrs: Vec<(String, String)> = headers
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                        .collect();
                    cap.lock().unwrap().push(MockCapturedRequest {
                        path: "/session".to_string(),
                        headers: hdrs,
                        body: body_json,
                    });
                    axum::Json(serde_json::json!({"id": "mock-session-id"}))
                }
            }),
        )
        .route(
            "/session/{id}/message",
            post(
                move |path: axum::extract::Path<String>,
                      headers: axum::http::HeaderMap,
                      body: String| {
                    let cap = cap_message.clone();
                    let session_id = path.0;
                    async move {
                        let body_json: serde_json::Value =
                            serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
                        let hdrs: Vec<(String, String)> = headers
                            .iter()
                            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                            .collect();
                        cap.lock().unwrap().push(MockCapturedRequest {
                            path: format!("/session/{}/message", session_id),
                            headers: hdrs,
                            body: body_json,
                        });
                        if error_on_message {
                            Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                        } else {
                            Ok(axum::Json(serde_json::json!({
                                "info": {
                                    "id": "msg-1",
                                    "role": "assistant",
                                    "finish": "stop"
                                },
                                "parts": [{"type": "text", "text": "Mock AI response"}]
                            })))
                        }
                    }
                },
            ),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    (port, captured, handle)
}

/// Create test config with an OpenCode backend credential ("test_opencode")
fn test_config_with_opencode_backend(gateway_port: u16, opencode_port: u16) -> Config {
    let mut backends = HashMap::new();
    backends.insert(
        "opencode_test".to_string(),
        BackendConfig {
            protocol: BackendProtocol::Opencode,
            inbound_url: None,
            base_url: Some(format!("http://127.0.0.1:{}", opencode_port)),
            token: "testuser:testpass".to_string(),
            poll_interval_ms: None,
            adapter_dir: None,
            port: None,
            active: true,
            config: Some(serde_json::json!({
                "model": {
                    "providerID": "test",
                    "modelID": "test-model"
                }
            })),
        },
    );
    Config {
        gateway: GatewayConfig {
            listen: format!("127.0.0.1:{}", gateway_port),
            admin_token: "test_admin_token".to_string(),
            default_backend: None,
            adapters_dir: "./adapters".to_string(),
            adapter_port_range: (19000, 19100),
            backends_dir: "./backends".to_string(),
            backend_port_range: (19200, 19300),
            file_cache: None,
            guardrails_dir: None,
        },
        auth: AuthConfig {
            send_token: "test_send_token".to_string(),
        },
        health_checks: HashMap::new(),
        backends,
        credentials: {
            let mut creds = HashMap::new();
            creds.insert(
                "test_opencode".to_string(),
                CredentialConfig {
                    adapter: "generic".to_string(),
                    token: "generic_token".to_string(),
                    active: true,
                    emergency: false,
                    route: serde_json::json!({"channel": "test_opencode"}),
                    config: None,
                    backend: Some("opencode_test".to_string()),
                },
            );
            creds
        },
    }
}

/// Full roundtrip: generic inbound → OpenCode mock → self-relay → WS client receives response
#[tokio::test]
async fn test_opencode_backend_full_roundtrip() {
    use futures_util::StreamExt;
    use tokio_tungstenite::connect_async;

    let (oc_port, captured, _oc_handle) = spawn_mock_opencode(false).await;
    let gw_port = find_available_port().await;
    let server = TestServer::new(test_config_with_opencode_backend(gw_port, oc_port)).await;
    let client = server.client();

    // Connect WebSocket to receive responses
    let ws_url = format!(
        "ws://127.0.0.1:{}/ws/chat/test_opencode/oc-rt-chat",
        server.port
    );
    let request = http::Request::builder()
        .uri(&ws_url)
        .header("Authorization", "Bearer generic_token")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("Sec-WebSocket-Version", "13")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Host", format!("127.0.0.1:{}", server.port))
        .body(())
        .unwrap();

    let (ws_stream, _) = connect_async(request)
        .await
        .expect("WebSocket connect failed");
    let (_write, mut read) = ws_stream.split();

    // Give WS connection time to register
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send message via generic inbound
    let resp = client
        .post(server.url("/api/v1/chat/test_opencode"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "Hello AI",
            "chat_id": "oc-rt-chat",
            "from": {"id": "user1", "display_name": "Test User"}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 202);

    // Wait for WebSocket message (background: inbound → OpenCode → self-relay → WS)
    let ws_msg = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(msg) = read.next().await {
            if let Ok(tokio_tungstenite::tungstenite::Message::Text(text)) = msg {
                return text.to_string();
            }
        }
        String::new()
    })
    .await
    .expect("Timed out waiting for WebSocket message");

    assert!(!ws_msg.is_empty(), "No WebSocket message received");
    let ws_body: serde_json::Value =
        serde_json::from_str(&ws_msg).expect("WS message is not valid JSON");
    assert_eq!(ws_body["text"], "Mock AI response");

    // Verify mock received both session creation and message
    let reqs = captured.lock().unwrap();
    assert!(
        reqs.iter().any(|r| r.path == "/session"),
        "Mock should have received session creation POST"
    );
    assert!(
        reqs.iter().any(|r| r.path.contains("/message")),
        "Mock should have received message POST"
    );
}

/// Session reuse: 2 messages to same chat_id → 1 session creation, 2 message sends
#[tokio::test]
async fn test_opencode_backend_session_reuse() {
    let (oc_port, captured, _oc_handle) = spawn_mock_opencode(false).await;
    let gw_port = find_available_port().await;
    let server = TestServer::new(test_config_with_opencode_backend(gw_port, oc_port)).await;
    let client = server.client();

    // Send first message
    let resp = client
        .post(server.url("/api/v1/chat/test_opencode"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "First message",
            "chat_id": "oc-reuse-chat",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);

    // Wait for first request to complete
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Send second message with same chat_id
    let resp = client
        .post(server.url("/api/v1/chat/test_opencode"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "Second message",
            "chat_id": "oc-reuse-chat",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);

    // Wait for second request to complete
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Verify: 1 session creation, 2 message sends
    let reqs = captured.lock().unwrap();
    let session_count = reqs.iter().filter(|r| r.path == "/session").count();
    let message_count = reqs.iter().filter(|r| r.path.contains("/message")).count();

    assert_eq!(session_count, 1, "Should create session only once");
    assert_eq!(message_count, 2, "Should send 2 messages");
}

/// Verify that different credentials with the same chat_id create separate OpenCode sessions
#[tokio::test]
async fn test_opencode_backend_session_isolation() {
    let (oc_port, captured, _oc_handle) = spawn_mock_opencode(false).await;
    let gw_port = find_available_port().await;

    // Create config with two credentials pointing to same OpenCode backend
    let mut config = test_config_with_opencode_backend(gw_port, oc_port);
    config.credentials.insert(
        "test_opencode_beta".to_string(),
        CredentialConfig {
            adapter: "generic".to_string(),
            token: "generic_token_beta".to_string(),
            active: true,
            emergency: false,
            route: serde_json::json!({"channel": "test_opencode_beta"}),
            config: None,
            backend: Some("opencode_test".to_string()),
        },
    );

    let server = TestServer::new(config).await;
    let client = server.client();

    // Send message from credential alpha
    let resp = client
        .post(server.url("/api/v1/chat/test_opencode"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "Message from alpha",
            "chat_id": "iso-shared-chat",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);

    // Wait for first request to complete
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Send message from credential beta (same chat_id)
    let resp = client
        .post(server.url("/api/v1/chat/test_opencode_beta"))
        .header("Authorization", "Bearer generic_token_beta")
        .json(&serde_json::json!({
            "text": "Message from beta",
            "chat_id": "iso-shared-chat",
            "from": {"id": "user2"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);

    // Wait for second request to complete
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Verify: 2 session creations (one per credential), 2 message sends
    let reqs = captured.lock().unwrap();
    let session_count = reqs.iter().filter(|r| r.path == "/session").count();
    let message_count = reqs.iter().filter(|r| r.path.contains("/message")).count();

    assert_eq!(
        session_count, 2,
        "Should create separate sessions for different credentials"
    );
    assert_eq!(message_count, 2, "Should send 2 messages");
}

/// Verify Authorization header sent to mock is Basic auth with base64("testuser:testpass")
#[tokio::test]
async fn test_opencode_backend_auth_basic() {
    let (oc_port, captured, _oc_handle) = spawn_mock_opencode(false).await;
    let gw_port = find_available_port().await;
    let server = TestServer::new(test_config_with_opencode_backend(gw_port, oc_port)).await;
    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/chat/test_opencode"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "Check auth",
            "chat_id": "oc-auth-chat",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);

    // Wait for background task
    tokio::time::sleep(Duration::from_millis(1000)).await;

    let reqs = captured.lock().unwrap();
    let session_req = reqs
        .iter()
        .find(|r| r.path == "/session")
        .expect("Session request not found");

    let auth_header = session_req
        .headers
        .iter()
        .find(|(k, _)| k == "authorization")
        .map(|(_, v)| v.as_str())
        .expect("Authorization header not found");

    // base64("testuser:testpass") = "dGVzdHVzZXI6dGVzdHBhc3M="
    assert_eq!(auth_header, "Basic dGVzdHVzZXI6dGVzdHBhc3M=");
}

/// Verify request body to /session/:id/message contains the model config
#[tokio::test]
async fn test_opencode_backend_model_config_sent() {
    let (oc_port, captured, _oc_handle) = spawn_mock_opencode(false).await;
    let gw_port = find_available_port().await;
    let server = TestServer::new(test_config_with_opencode_backend(gw_port, oc_port)).await;
    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/chat/test_opencode"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "Check model config",
            "chat_id": "oc-model-chat",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);

    // Wait for background task
    tokio::time::sleep(Duration::from_millis(1000)).await;

    let reqs = captured.lock().unwrap();
    let msg_req = reqs
        .iter()
        .find(|r| r.path.contains("/message"))
        .expect("Message request not found");

    assert_eq!(msg_req.body["model"]["providerID"], "test");
    assert_eq!(msg_req.body["model"]["modelID"], "test-model");
}

/// Mock returns 500 for message — gateway still returns 202 and doesn't crash
#[tokio::test]
async fn test_opencode_backend_error_response() {
    let (oc_port, _captured, _oc_handle) = spawn_mock_opencode(true).await;
    let gw_port = find_available_port().await;
    let server = TestServer::new(test_config_with_opencode_backend(gw_port, oc_port)).await;
    let client = server.client();

    // Send message — gateway returns 202 (fire-and-forget), backend error is internal
    let resp = client
        .post(server.url("/api/v1/chat/test_opencode"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "Will fail",
            "chat_id": "oc-err-chat",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);

    // Wait for background task to process (and error internally)
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Verify gateway is still alive and responsive
    let health_resp = client.get(server.url("/health")).send().await.unwrap();
    assert!(health_resp.status().is_success());
}

// ============================================================================
// Guardrail Integration Tests
// ============================================================================

/// Helper: create a guardrails dir with a block rule that matches text containing "blocked"
fn guardrail_block_rule_dir() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    let rule = r#"{"name":"block-test","expression":"message.text.contains(\"blocked\")","action":"block","reject_message":"Content blocked by guardrail"}"#;
    std::fs::write(dir.path().join("01-block.json"), rule).unwrap();
    dir
}

/// POST inbound with block rule + matching text → HTTP 403 + error body
#[tokio::test]
async fn test_guardrail_blocks_matching_inbound() {
    let guardrails_dir = guardrail_block_rule_dir();
    let port = find_available_port().await;
    let mut config = test_config(port);
    config.gateway.guardrails_dir = Some(guardrails_dir.path().to_string_lossy().into_owned());

    let server = TestServer::new(config).await;
    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/chat/test_generic"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "This message is blocked by rule",
            "chat_id": "guardrail-test",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 403);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "Content blocked by guardrail");
}

/// POST inbound with block rule + non-matching text → HTTP 202
#[tokio::test]
async fn test_guardrail_allows_non_matching_inbound() {
    let guardrails_dir = guardrail_block_rule_dir();
    let port = find_available_port().await;
    let mut config = test_config(port);
    config.gateway.guardrails_dir = Some(guardrails_dir.path().to_string_lossy().into_owned());

    let server = TestServer::new(config).await;
    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/chat/test_generic"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "This is a perfectly fine message",
            "chat_id": "guardrail-test",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 202);
}

/// POST inbound with no rules → HTTP 202 (passthrough)
#[tokio::test]
async fn test_guardrail_passthrough_no_rules() {
    let port = find_available_port().await;
    let config = test_config(port); // guardrails_dir is None

    let server = TestServer::new(config).await;
    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/chat/test_generic"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "Any message at all",
            "chat_id": "guardrail-test",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 202);
}

/// Generic adapter chat_inbound with block rule → HTTP 403
#[tokio::test]
async fn test_guardrail_blocks_generic_chat_inbound() {
    let guardrails_dir = guardrail_block_rule_dir();
    let port = find_available_port().await;
    let mut config = test_config(port);
    config.gateway.guardrails_dir = Some(guardrails_dir.path().to_string_lossy().into_owned());

    let server = TestServer::new(config).await;
    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/chat/test_generic"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "message containing blocked word",
            "chat_id": "guardrail-chat",
            "from": {"id": "user2", "display_name": "Test User"}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 403);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("blocked"));
}

/// (a) POST inbound via /api/v1/adapter/inbound with block rule + matching text → HTTP 403
#[tokio::test]
async fn test_guardrail_blocks_adapter_inbound_endpoint() {
    let guardrails_dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        guardrails_dir.path().join("01-block.json"),
        r#"{"name":"block-adapter","expression":"message.text.contains(\"blocked\")","action":"block","reject_message":"Blocked via adapter endpoint"}"#,
    )
    .unwrap();

    let adapters_dir = tempfile::TempDir::new().unwrap();
    let test_adapter_dir = adapters_dir.path().join("test_adapter");
    std::fs::create_dir(&test_adapter_dir).unwrap();
    std::fs::write(
        test_adapter_dir.join("adapter.json"),
        r#"{"name":"test_adapter","version":"1.0","command":"sleep","args":["60"]}"#,
    )
    .unwrap();

    let port = find_available_port().await;
    let mut config = test_config(port);
    config.gateway.guardrails_dir = Some(guardrails_dir.path().to_string_lossy().into_owned());
    config.gateway.adapters_dir = adapters_dir.path().to_string_lossy().into_owned();
    config.credentials.insert(
        "test_external".to_string(),
        CredentialConfig {
            adapter: "test_adapter".to_string(),
            token: "ext_token".to_string(),
            active: true,
            emergency: false,
            route: serde_json::json!({"channel": "external"}),
            config: None,
            backend: None,
        },
    );

    let server = TestServer::new(config).await;

    let (instance_id, _) = server
        .state
        .adapter_manager
        .spawn("test_external", "test_adapter", "ext_token", None)
        .await
        .unwrap();

    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/adapter/inbound"))
        .json(&serde_json::json!({
            "instance_id": instance_id,
            "chat_id": "adapter-chat",
            "message_id": "msg-1",
            "text": "This message is blocked by rule",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 403);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "Blocked via adapter endpoint");

    let _ = server.state.adapter_manager.stop("test_external").await;
}

/// (b) POST inbound via generic chat endpoint with block rule → HTTP 403 + reject_message body
#[tokio::test]
async fn test_guardrail_blocks_generic_chat_with_custom_reject_message() {
    let guardrails_dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        guardrails_dir.path().join("01-block.json"),
        r#"{"name":"custom-reject","expression":"message.text.contains(\"forbidden\")","action":"block","reject_message":"Custom rejection: content policy violation"}"#,
    )
    .unwrap();

    let port = find_available_port().await;
    let mut config = test_config(port);
    config.gateway.guardrails_dir = Some(guardrails_dir.path().to_string_lossy().into_owned());

    let server = TestServer::new(config).await;
    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/chat/test_generic"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "This contains forbidden content",
            "chat_id": "reject-test",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 403);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "Custom rejection: content policy violation");
}

/// (c) Allowed when no rule matches the message → HTTP 202
#[tokio::test]
async fn test_guardrail_allows_when_expression_does_not_match() {
    let guardrails_dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        guardrails_dir.path().join("01-block.json"),
        r#"{"name":"block-secret","expression":"message.text.contains(\"secret_keyword\")","action":"block","reject_message":"Blocked"}"#,
    )
    .unwrap();

    let port = find_available_port().await;
    let mut config = test_config(port);
    config.gateway.guardrails_dir = Some(guardrails_dir.path().to_string_lossy().into_owned());

    let server = TestServer::new(config).await;
    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/chat/test_generic"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "This is a perfectly normal message with no matches",
            "chat_id": "allow-test",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 202);
}

/// (d) Multiple rules with short-circuit: first block rule stops evaluation
#[tokio::test]
async fn test_guardrail_short_circuits_on_first_matching_block_rule() {
    let guardrails_dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        guardrails_dir.path().join("01-first-rule.json"),
        r#"{"name":"first-rule","expression":"message.text.contains(\"badword\")","action":"block","reject_message":"Blocked by FIRST rule"}"#,
    )
    .unwrap();
    std::fs::write(
        guardrails_dir.path().join("02-second-rule.json"),
        r#"{"name":"second-rule","expression":"message.text.contains(\"badword\")","action":"block","reject_message":"Blocked by SECOND rule"}"#,
    )
    .unwrap();

    let port = find_available_port().await;
    let mut config = test_config(port);
    config.gateway.guardrails_dir = Some(guardrails_dir.path().to_string_lossy().into_owned());

    let server = TestServer::new(config).await;
    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/chat/test_generic"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "message with badword in it",
            "chat_id": "shortcircuit-test",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 403);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "Blocked by FIRST rule");
}

/// (e) Fail-open on CEL evaluation error (on_error=allow) → HTTP 202
#[tokio::test]
async fn test_guardrail_fail_open_on_cel_error() {
    let guardrails_dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        guardrails_dir.path().join("01-error-rule.json"),
        r#"{"name":"error-rule","expression":"message.nonexistent_deeply_nested.field == true","action":"block","on_error":"allow","reject_message":"Should not see this"}"#,
    )
    .unwrap();

    let port = find_available_port().await;
    let mut config = test_config(port);
    config.gateway.guardrails_dir = Some(guardrails_dir.path().to_string_lossy().into_owned());

    let server = TestServer::new(config).await;
    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/chat/test_generic"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "Any message triggers the error rule",
            "chat_id": "failopen-test",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 202);
}

/// (f) Disabled rule is ignored → HTTP 202 (message passes through)
#[tokio::test]
async fn test_guardrail_disabled_rule_ignored() {
    let guardrails_dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        guardrails_dir.path().join("01-disabled.json"),
        r#"{"name":"disabled-blocker","expression":"true","action":"block","enabled":false,"reject_message":"Should never block"}"#,
    )
    .unwrap();

    let port = find_available_port().await;
    let mut config = test_config(port);
    config.gateway.guardrails_dir = Some(guardrails_dir.path().to_string_lossy().into_owned());

    let server = TestServer::new(config).await;
    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/chat/test_generic"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "This should pass through despite block-all rule",
            "chat_id": "disabled-test",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 202);
}

/// (g) Empty guardrails directory = passthrough → HTTP 202
#[tokio::test]
async fn test_guardrail_empty_dir_passthrough() {
    let guardrails_dir = tempfile::TempDir::new().unwrap();

    let port = find_available_port().await;
    let mut config = test_config(port);
    config.gateway.guardrails_dir = Some(guardrails_dir.path().to_string_lossy().into_owned());

    let server = TestServer::new(config).await;
    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/chat/test_generic"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "Any message at all in empty guardrails dir",
            "chat_id": "empty-dir-test",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 202);
}

/// (h) Regex matching via matches() → HTTP 403 when pattern matches
#[tokio::test]
async fn test_guardrail_regex_matches_blocks() {
    let guardrails_dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        guardrails_dir.path().join("01-regex.json"),
        r#"{"name":"no-passwords","expression":"message.text.matches('(?i)password')","action":"block","reject_message":"Message contains sensitive content"}"#,
    )
    .unwrap();

    let port = find_available_port().await;
    let mut config = test_config(port);
    config.gateway.guardrails_dir = Some(guardrails_dir.path().to_string_lossy().into_owned());

    let server = TestServer::new(config).await;
    let client = server.client();

    let resp = client
        .post(server.url("/api/v1/chat/test_generic"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "My Password is hunter2",
            "chat_id": "regex-test",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 403);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "Message contains sensitive content");

    let resp2 = client
        .post(server.url("/api/v1/chat/test_generic"))
        .header("Authorization", "Bearer generic_token")
        .json(&serde_json::json!({
            "text": "This message is completely safe",
            "chat_id": "regex-test-2",
            "from": {"id": "user1"}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp2.status(), 202);
}
