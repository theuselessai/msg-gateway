//! `plit credentials` — admin CRUD operations for gateway credentials.

use anyhow::Result;

use crate::client::{CreateCredentialRequest, GatewayClient};
use crate::output;

use super::Context;

pub async fn run(ctx: &Context, cmd: crate::CredentialCommands) -> Result<()> {
    let admin_token = ctx.require_admin_token()?;
    let client = GatewayClient::new(&ctx.gateway_url);

    match cmd {
        crate::CredentialCommands::List => {
            let creds = client.list_credentials(admin_token).await?;
            output::print_result(&creds, ctx.json_output);
        }

        crate::CredentialCommands::Create {
            id,
            adapter,
            token,
            backend,
            route,
            config,
            active,
        } => {
            let route_value = route
                .map(|r| serde_json::from_str(&r))
                .transpose()
                .map_err(|e| anyhow::anyhow!("Invalid --route JSON: {}", e))?;

            let config_value = config
                .map(|c| serde_json::from_str(&c))
                .transpose()
                .map_err(|e| anyhow::anyhow!("Invalid --config JSON: {}", e))?;

            let req = CreateCredentialRequest {
                id,
                adapter,
                token,
                backend,
                route: route_value,
                config: config_value,
                active,
            };

            let result = client.create_credential(admin_token, &req).await?;
            output::print_result(&result, ctx.json_output);
        }

        crate::CredentialCommands::Activate { id } => {
            let result = client.activate_credential(admin_token, &id).await?;
            output::print_result(&result, ctx.json_output);
        }

        crate::CredentialCommands::Deactivate { id } => {
            let result = client.deactivate_credential(admin_token, &id).await?;
            output::print_result(&result, ctx.json_output);
        }
    }

    Ok(())
}
