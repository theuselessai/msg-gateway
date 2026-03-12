pub mod chat;
pub mod credentials;
pub mod health;
pub mod init;
pub mod listen;
pub mod send;
pub mod start;
pub mod stop;
pub mod uninstall;

/// Shared context passed to all commands.
pub struct Context {
    pub gateway_url: String,
    pub token: Option<String>,
    pub admin_token: Option<String>,
    pub json_output: bool,
}

impl Context {
    /// Get the credential token or bail with a helpful error.
    pub fn require_token(&self) -> anyhow::Result<&str> {
        self.token.as_deref().ok_or_else(|| {
            anyhow::anyhow!("Missing credential token. Set --token or GATEWAY_TOKEN")
        })
    }

    /// Get the admin token or bail with a helpful error.
    pub fn require_admin_token(&self) -> anyhow::Result<&str> {
        self.admin_token.as_deref().ok_or_else(|| {
            anyhow::anyhow!("Missing admin token. Set --admin-token or GATEWAY_ADMIN_TOKEN")
        })
    }
}
