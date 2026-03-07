//! Integration tests for msg-gateway
//!
//! These tests spawn a real gateway server and mock backend to test the full flow.

use msg_gateway::adapter::AdapterInstanceManager;
use msg_gateway::config::{
    AuthConfig, BackendProtocol, Config, CredentialConfig, GatewayConfig, TargetConfig,
};
use msg_gateway::manager::CredentialManager;
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

/// Helper to find an available port
async fn find_available_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

#[tokio::test]
async fn test_health_endpoint() {
    let port = find_available_port().await;
    let config = test_config(port);

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

    // Spawn server in background
    let server_handle = tokio::spawn(async move {
        let _ = server_future.await;
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Test health endpoint
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{}/health", port))
        .send()
        .await
        .expect("Failed to send request");

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");

    // Cleanup
    server_handle.abort();
    drop(state);
}

#[tokio::test]
async fn test_admin_auth() {
    let port = find_available_port().await;
    let config = test_config(port);
    let admin_token = config.gateway.admin_token.clone();

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

    let server_handle = tokio::spawn(async move {
        let _ = server_future.await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = reqwest::Client::new();

    // Test without auth - should fail
    let resp = client
        .get(format!("http://127.0.0.1:{}/admin/health", port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Test with wrong token - should fail
    let resp = client
        .get(format!("http://127.0.0.1:{}/admin/health", port))
        .header("Authorization", "Bearer wrong_token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Test with correct token - should succeed
    let resp = client
        .get(format!("http://127.0.0.1:{}/admin/health", port))
        .header("Authorization", format!("Bearer {}", admin_token))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    server_handle.abort();
    drop(state);
}

#[tokio::test]
async fn test_send_auth() {
    let port = find_available_port().await;
    let config = test_config(port);
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

    let server_handle = tokio::spawn(async move {
        let _ = server_future.await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = reqwest::Client::new();

    // Test without auth - should fail
    let resp = client
        .post(format!("http://127.0.0.1:{}/api/v1/send", port))
        .json(&serde_json::json!({
            "credential_id": "test_generic",
            "chat_id": "chat1",
            "text": "Hello"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Test with correct token - should succeed (even if no WS client connected)
    let resp = client
        .post(format!("http://127.0.0.1:{}/api/v1/send", port))
        .header("Authorization", format!("Bearer {}", send_token))
        .json(&serde_json::json!({
            "credential_id": "test_generic",
            "chat_id": "chat1",
            "text": "Hello"
        }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    server_handle.abort();
    drop(state);
}

#[tokio::test]
async fn test_list_credentials() {
    let port = find_available_port().await;
    let config = test_config(port);
    let admin_token = config.gateway.admin_token.clone();

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

    let server_handle = tokio::spawn(async move {
        let _ = server_future.await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{}/admin/credentials", port))
        .header("Authorization", format!("Bearer {}", admin_token))
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

    server_handle.abort();
    drop(state);
}
