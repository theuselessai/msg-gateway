//! `plit send` — one-shot message send.
//!
//! Reads text from --text flag or stdin, POSTs to the generic chat endpoint,
//! prints the result as JSON.

use anyhow::Result;
use std::io::Read;

use crate::client::GatewayClient;
use crate::output;

use super::Context;

pub async fn run(
    ctx: &Context,
    credential_id: &str,
    chat_id: &str,
    text: Option<&str>,
    user_id: &str,
) -> Result<()> {
    let token = ctx.require_token()?;
    let client = GatewayClient::new(&ctx.gateway_url);

    // Get text from --text flag or stdin
    let message_text = match text {
        Some(t) => t.to_string(),
        None => {
            if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                output::status("Reading message from stdin (Ctrl+D to send):");
            }
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| anyhow::anyhow!("Failed to read stdin: {}", e))?;
            let trimmed = buf.trim_end().to_string();
            if trimmed.is_empty() {
                anyhow::bail!("No message text provided. Use --text or pipe to stdin.");
            }
            trimmed
        }
    };

    let resp = client
        .send_chat_message(credential_id, chat_id, &message_text, user_id, token)
        .await?;

    output::print_result(&resp, ctx.json_output);

    Ok(())
}
