//! WebSocket integration test
//! 
//! Run with: cargo test --test ws_test -- --nocapture

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::test]
async fn test_websocket_flow() {
    // This test requires the server to be running
    // Start server: GATEWAY_CONFIG=config.example.json cargo run
    
    let ws_url = "ws://127.0.0.1:8080/ws/chat/generic_chat/test_session";
    
    // Connect with auth header
    let request = http::Request::builder()
        .uri(ws_url)
        .header("Authorization", "Bearer chat123")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("Sec-WebSocket-Version", "13")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Host", "127.0.0.1:8080")
        .body(())
        .unwrap();

    let (ws_stream, _) = connect_async(request).await.expect("Failed to connect");
    println!("WebSocket connected!");

    let (mut write, mut read) = ws_stream.split();

    // Spawn task to read messages
    let read_task = tokio::spawn(async move {
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    println!("Received: {}", text);
                    return text.to_string();
                }
                Ok(Message::Close(_)) => {
                    println!("Connection closed");
                    break;
                }
                Err(e) => {
                    println!("Error: {}", e);
                    break;
                }
                _ => {}
            }
        }
        String::new()
    });

    // Give a moment for WebSocket to register
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Send a message via HTTP to trigger outbound
    let client = reqwest::Client::new();
    let resp = client
        .post("http://127.0.0.1:8080/api/v1/send")
        .header("Authorization", "Bearer send123")
        .json(&serde_json::json!({
            "credential_id": "generic_chat",
            "chat_id": "test_session",
            "text": "Hello from test!"
        }))
        .send()
        .await
        .expect("Failed to send");

    println!("Send response: {:?}", resp.status());

    // Wait for message with timeout
    let received = tokio::time::timeout(
        tokio::time::Duration::from_secs(2),
        read_task
    ).await;

    match received {
        Ok(Ok(msg)) => {
            println!("Test passed! Received: {}", msg);
            assert!(msg.contains("Hello from test!"));
        }
        _ => {
            panic!("Did not receive message via WebSocket");
        }
    }
}
