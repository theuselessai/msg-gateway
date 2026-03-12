mod config;
mod install;
mod prereqs;
mod prompts;
mod tokens;

use anyhow::Result;
use dialoguer::Confirm;

use crate::output;

pub async fn run() -> Result<()> {
    output::status("plit init — setting up Pipelit + Gateway\n");

    // 1. Re-run detection
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

    // 2. Prereqs
    output::status("Checking prerequisites...");
    let env = prereqs::check_all()?;
    output::status("");

    // 3. Clone + venv + deps
    output::status("Setting up Pipelit...");

    if pipelit_exists && !config_exists {
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

    // 4-13. Prompts (ports, admin, redis, base url, LLM)
    output::status("Configuration:");
    let inputs = prompts::collect().await?;
    output::status("");

    // 14. Generate tokens
    output::status("Generating shared tokens...");
    let shared_tokens = tokens::generate();
    output::status("  ✓ Generated 3 shared tokens");
    output::status("");

    // 15. Write .env (needed for migrations)
    output::status("Writing environment file...");
    config::write_dot_env(&inputs, &shared_tokens)?;
    output::status("");

    // 16. Run migrations
    output::status("Database setup...");
    let env_path = config::dot_env_path()?;
    install::run_migrations(&env_path).await?;
    output::status("");

    // 17. CLI setup (admin user + conf.json + workspace + rootfs)
    output::status("Platform setup...");
    install::run_cli_setup(&inputs, &env, &env_path).await?;
    output::status("");

    // 18. Apply fixture
    output::status("Creating default workflow...");
    let fixture = install::run_apply_fixture(&inputs, &env_path).await?;
    output::status("");

    // 19. Write config.json (with credential route from fixture)
    output::status("Writing gateway configuration...");
    config::write_gateway_config(&inputs, &shared_tokens, &fixture)?;
    output::status("");

    // 20. Done
    output::status("Setup complete!");
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
