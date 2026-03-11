//! `plit health` — check gateway health status.

use anyhow::Result;

use crate::client::GatewayClient;
use crate::output;

use super::Context;

/// Returns `Ok(true)` when healthy, `Ok(false)` when unhealthy.
pub async fn run(ctx: &Context) -> Result<bool> {
    let client = GatewayClient::new(&ctx.gateway_url);

    let health = client.health().await?;

    output::print_result(&health, ctx.json_output);

    Ok(health.status == "ok")
}
