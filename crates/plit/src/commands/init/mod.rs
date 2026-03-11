//! `plit init` — interactive setup wizard for the Pipelit + Gateway stack.
//!
//! Bootstraps a complete installation from scratch:
//! 1. Check prerequisites (python3, pip3, redis-server, git)
//! 2. Clone Pipelit, create venv, install deps
//! 3. Prompt for ports and admin credentials
//! 4. Generate shared tokens
//! 5. Write config files
//! 6. Run database migrations
//! 7. Create admin user

mod config;
mod install;
mod prereqs;
mod prompts;
mod tokens;

use anyhow::Result;
use dialoguer::Confirm;

use crate::output;

/// Run the `plit init` wizard.
pub async fn run() -> Result<()> {
    output::status("plit init — setting up Pipelit + Gateway\n");

    // -----------------------------------------------------------------------
    // 1. Re-run detection
    // -----------------------------------------------------------------------
    let config_exists = config::config_json_path()?.exists();
    let pipelit_exists = config::pipelit_dir()?.exists();

    if config_exists {
        output::status("Existing installation detected.");
        let reset = Confirm::new()
            .with_prompt("Reset and reconfigure?")
            .default(false)
            .interact()?;

        if !reset {
            output::status("Exiting. Your existing configuration is unchanged.");
            return Ok(());
        }
        output::status("");
    }

    // -----------------------------------------------------------------------
    // 2. Check prerequisites
    // -----------------------------------------------------------------------
    output::status("Checking prerequisites...");
    prereqs::check_all()?;
    output::status("");

    // -----------------------------------------------------------------------
    // 3. Clone + install Pipelit
    // -----------------------------------------------------------------------
    output::status("Setting up Pipelit...");

    if pipelit_exists && !config_exists {
        // Pipelit dir exists but no config — possibly a partial install.
        // Offer to re-clone.
        let reclone = Confirm::new()
            .with_prompt("Pipelit directory already exists. Re-clone from scratch?")
            .default(false)
            .interact()?;

        if reclone {
            let pipelit_dir = config::pipelit_dir()?;
            std::fs::remove_dir_all(&pipelit_dir)?;
            install::clone_pipelit().await?;
        }
    } else {
        install::clone_pipelit().await?;
    }

    install::create_venv().await?;
    install::install_deps().await?;
    output::status("");

    // -----------------------------------------------------------------------
    // 4. Prompt for ports + admin credentials
    // -----------------------------------------------------------------------
    output::status("Configuration:");
    let inputs = prompts::collect()?;
    output::status("");

    // -----------------------------------------------------------------------
    // 5. Generate tokens
    // -----------------------------------------------------------------------
    output::status("Generating shared tokens...");
    let shared_tokens = tokens::generate();
    output::status("  ✓ Generated 3 shared tokens");
    output::status("");

    // -----------------------------------------------------------------------
    // 6. Write config files (.env first — needed for migrations)
    // -----------------------------------------------------------------------
    output::status("Writing configuration files...");
    config::write_configs(&inputs, &shared_tokens)?;
    output::status("");

    // -----------------------------------------------------------------------
    // 7. Run migrations
    // -----------------------------------------------------------------------
    output::status("Database setup...");
    let env_path = config::dot_env_path()?;
    install::run_migrations(&env_path).await?;
    output::status("");

    // -----------------------------------------------------------------------
    // 8. Create admin user
    // -----------------------------------------------------------------------
    output::status("Admin user setup...");
    install::create_admin_user(
        inputs.pipelit_port,
        &inputs.admin_username,
        &inputs.admin_password,
        &env_path,
    )
    .await?;
    output::status("");

    // -----------------------------------------------------------------------
    // 9. Success
    // -----------------------------------------------------------------------
    output::status("Setup complete! 🎉");
    output::status("");
    output::status(&format!(
        "  Config:  {}",
        config::config_json_path()?.display()
    ));
    output::status(&format!("  Env:     {}", config::dot_env_path()?.display()));
    output::status(&format!("  Pipelit: {}", config::pipelit_dir()?.display()));
    output::status("");
    output::status("Run `plit start` to launch.");

    Ok(())
}
