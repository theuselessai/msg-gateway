//! `plit chat` — interactive REPL combining send + listen.
//!
//! Connects WebSocket first, then reads user input line by line,
//! sends each line as a chat message, and displays responses as they arrive.

use anyhow::Result;
use futures_util::StreamExt;
use std::io::Write;
use tokio::io::AsyncBufReadExt;
use tokio_util::sync::CancellationToken;

use crate::client::GatewayClient;
use crate::output;

use super::Context;

const WS_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

pub async fn run(ctx: &Context, credential_id: &str, chat_id: &str, user_id: &str) -> Result<()> {
    let token = ctx.require_token()?.to_string();
    let gateway_url = ctx.gateway_url.clone();
    let json_output = ctx.json_output;
    let credential_id = credential_id.to_string();
    let chat_id = chat_id.to_string();
    let user_id = user_id.to_string();

    let client = GatewayClient::new(&gateway_url);

    output::status(&format!("Connecting to gateway at {}...", client.base_url));

    let mut ws_stream = client.connect_ws(&credential_id, &chat_id, &token).await?;

    output::status(&format!(
        "Connected. Chat session: credential={}, chat_id={}",
        credential_id, chat_id
    ));
    output::status("Type your message and press Enter. Ctrl+C to exit.\n");

    let cancel = CancellationToken::new();
    let cancel_ws = cancel.clone();

    let ws_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel_ws.cancelled() => break,
                msg = ws_stream.next() => {
                    match msg {
                        Some(Ok(msg)) => {
                            if json_output {
                                output::print_jsonl(&msg);
                            } else {
                                println!("\n< {}", msg.text);
                                if !msg.file_urls.is_empty() {
                                    for url in &msg.file_urls {
                                        println!("  [file] {}", url);
                                    }
                                }
                                print!("> ");
                                let _ = std::io::stdout().flush();
                            }
                        }
                        Some(Err(e)) => {
                            output::error(&format!("WebSocket error: {}", e));
                            break;
                        }
                        None => break,
                    }
                }
            }
        }
    });

    let stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();

    let send_client = GatewayClient::new(&gateway_url);

    loop {
        if !json_output {
            print!("> ");
            let _ = std::io::stdout().flush();
        }

        let line = match lines.next_line().await? {
            Some(line) => line,
            None => break,
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match send_client
            .send_chat_message(&credential_id, &chat_id, trimmed, &user_id, &token)
            .await
        {
            Ok(resp) => {
                if json_output {
                    output::print_jsonl(&resp);
                }
            }
            Err(e) => {
                output::error(&format!("Send failed: {}", e));
            }
        }
    }

    // Signal WS task to stop and wait for it to drain remaining messages
    cancel.cancel();
    match tokio::time::timeout(WS_DRAIN_TIMEOUT, ws_task).await {
        Ok(result) => {
            if let Err(e) = result {
                if !e.is_cancelled() {
                    output::error(&format!("WebSocket task error: {}", e));
                }
            }
        }
        Err(_) => {
            output::status("WebSocket drain timed out, closing.");
        }
    }

    output::status("\nChat session ended.");
    Ok(())
}
