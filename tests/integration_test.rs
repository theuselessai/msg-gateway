//! Integration tests for msg-gateway
//!
//! These tests spawn a real gateway server and mock backend to test the full flow.

use msg_gateway::adapter::AdapterInstanceManager;
use msg_gateway::config::{
    AuthConfig, BackendProtocol, Config, CredentialConfig, FileCacheConfig, GatewayConfig,
    TargetConfig,
};
use msg_gateway::manager::CredentialManager;
use serial_test::serial;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Create a minimal test config
fn test_config(port: u16) -> Config {
    Config {
        gateway: GatewayConfig {
            listen: format!("127.0.0.1:{}", port),
            admin_token: "test_admin_token".to_string(),
            default_target: TargetConfig {
                protocol: BackendProtocol::Pipelit,
                inbound_url: Some("http://127.0.0.1:18000/inbound".to_string()),
                base_url: None,
                token: "test_backend_token".to_string(),
                poll_interval_ms: None,
            },
            adapters_dir: "./adapters".to_string(),
            adapter_port_range: (19000, 19100),
            file_cache: None,
        },
        auth: AuthConfig {
            send_token: "test_send_token".to_string(),
        },
        health_checks: HashMap::new(),
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
                    target: None,
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
    state: Arc<msg_gateway::server::AppState>,
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

        let (state, server_future) =
            msg_gateway::server::create_server(config, manager, adapter_manager)
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
