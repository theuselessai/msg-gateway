//! Interactive user prompts for `plit init`.
//!
//! Uses `dialoguer` for a polished terminal experience with defaults,
//! validation, and password confirmation.

use anyhow::{Result, bail};
use dialoguer::{Input, Password, theme::ColorfulTheme};

/// Values collected from the user during interactive setup.
pub struct UserInputs {
    pub gateway_port: u16,
    pub pipelit_port: u16,
    pub admin_username: String,
    pub admin_password: String,
}

/// Run all interactive prompts and return collected values.
pub fn collect() -> Result<UserInputs> {
    let theme = ColorfulTheme::default();

    let gateway_port: u16 = Input::with_theme(&theme)
        .with_prompt("Gateway port")
        .default(8080)
        .interact_text()?;

    let pipelit_port: u16 = Input::with_theme(&theme)
        .with_prompt("Pipelit port")
        .default(8000)
        .interact_text()?;

    if gateway_port == pipelit_port {
        bail!(
            "Gateway port ({}) and Pipelit port ({}) must be different",
            gateway_port,
            pipelit_port
        );
    }

    let admin_username: String = Input::with_theme(&theme)
        .with_prompt("Admin username")
        .default("admin".to_string())
        .interact_text()?;

    if admin_username.trim().is_empty() {
        bail!("Admin username cannot be empty");
    }

    let admin_password = Password::with_theme(&theme)
        .with_prompt("Admin password")
        .with_confirmation("Confirm password", "Passwords don't match")
        .interact()?;

    if admin_password.is_empty() {
        bail!("Admin password cannot be empty");
    }

    Ok(UserInputs {
        gateway_port,
        pipelit_port,
        admin_username,
        admin_password,
    })
}
