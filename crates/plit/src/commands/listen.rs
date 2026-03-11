//! `plit listen` — stream outbound messages from WebSocket as JSONL.
//!
//! Connects to the gateway WebSocket and prints each message as a JSON line
//! to stdout. Designed for piping to jq, agent consumption, or scripting.

use anyhow::Result;
use futures_util::StreamExt;

use crate::client::GatewayClient;
use crate::output;

use super::Context;

pub async fn run(ctx: &Context, credential_id: &str, chat_id: &str) -> Result<()> {
    let token = ctx.require_token()?;
    let client = GatewayClient::new(&ctx.gateway_url);

    output::status(&format!(
        "Connecting to {}/ws/chat/{}/{}",
        client.base_url, credential_id, chat_id
    ));

    let mut stream = client.connect_ws(credential_id, chat_id, token).await?;

    output::status("Connected. Listening for messages...");

    while let Some(result) = stream.next().await {
        match result {
            Ok(msg) => {
                output::print_jsonl(&msg);
            }
            Err(e) => {
                return Err(e);
            }
        }
    }

    output::status("WebSocket disconnected.");
    Ok(())
}
