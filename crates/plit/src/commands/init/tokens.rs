//! Token generation for `plit init`.
//!
//! Generates cryptographically random UUIDs for the shared tokens
//! that connect the gateway and Pipelit.

use uuid::Uuid;

/// The three shared tokens needed for gateway ↔ Pipelit integration.
pub struct SharedTokens {
    /// Gateway admin token — used for `gateway.admin_token` / `GATEWAY_ADMIN_TOKEN`
    pub admin_token: String,
    /// Send token — used for `auth.send_token` / `GATEWAY_SEND_TOKEN`
    pub send_token: String,
    /// Inbound token — used for `backends.pipelit.token` / `GATEWAY_INBOUND_TOKEN`
    pub inbound_token: String,
}

/// Generate three fresh cryptographically random UUID tokens.
pub fn generate() -> SharedTokens {
    SharedTokens {
        admin_token: Uuid::new_v4().to_string(),
        send_token: Uuid::new_v4().to_string(),
        inbound_token: Uuid::new_v4().to_string(),
    }
}
