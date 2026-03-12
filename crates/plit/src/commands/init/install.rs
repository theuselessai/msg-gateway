use std::path::Path;

use anyhow::{Context, Result, bail};
use tokio::process::Command;

use super::config;
use crate::output;

pub async fn clone_pipelit() -> Result<()> {
    let pipelit_dir = config::pipelit_dir()?;

    if pipelit_dir.exists() {
        output::status(&format!(
            "  • Pipelit directory already exists at {}",
            pipelit_dir.display()
        ));
        return Ok(());
    }

    let parent = pipelit_dir
        .parent()
        .context("Invalid pipelit directory path")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create directory: {}", parent.display()))?;

    output::status("  • Cloning Pipelit repository...");

    let status = Command::new("git")
        .args(["clone", "https://github.com/theuselessai/Pipelit.git"])
        .arg(&pipelit_dir)
        .status()
        .await
        .context("Failed to run git clone")?;

    if !status.success() {
        bail!("git clone failed with exit code {}", status);
    }

    output::status("  ✓ Cloned Pipelit");
    Ok(())
}

pub async fn create_venv() -> Result<()> {
    let venv_dir = config::venv_dir()?;

    if venv_dir.join("bin").join("python").exists() {
        output::status("  • Virtualenv already exists");
        return Ok(());
    }

    output::status("  • Creating Python virtualenv...");

    let status = Command::new("python3")
        .args(["-m", "venv"])
        .arg(&venv_dir)
        .status()
        .await
        .context("Failed to create virtualenv")?;

    if !status.success() {
        bail!("python3 -m venv failed with exit code {}", status);
    }

    output::status("  ✓ Created virtualenv");
    Ok(())
}

pub async fn install_deps() -> Result<()> {
    let venv_dir = config::venv_dir()?;
    let pipelit_dir = config::pipelit_dir()?;
    let pip = venv_dir.join("bin").join("pip");
    let requirements = pipelit_dir.join("platform").join("requirements.txt");

    if !requirements.exists() {
        bail!(
            "requirements.txt not found at {}. Is Pipelit cloned correctly?",
            requirements.display()
        );
    }

    output::status("  • Installing Python dependencies (this may take a minute)...");

    let status = Command::new(&pip)
        .args(["install", "-r"])
        .arg(&requirements)
        .status()
        .await
        .context("Failed to run pip install")?;

    if !status.success() {
        bail!("pip install failed with exit code {}", status);
    }

    output::status("  ✓ Installed dependencies");
    Ok(())
}

pub async fn run_migrations(env_path: &Path) -> Result<()> {
    let venv_dir = config::venv_dir()?;
    let pipelit_dir = config::pipelit_dir()?;
    let alembic = venv_dir.join("bin").join("alembic");

    if !alembic.exists() {
        output::status("  • Alembic not found in venv, skipping migrations");
        return Ok(());
    }

    output::status("  • Running database migrations...");

    let env_vars = parse_env_file(env_path)?;

    let mut cmd = Command::new(&alembic);
    cmd.args(["upgrade", "head"])
        .current_dir(pipelit_dir.join("platform"));

    for (key, value) in &env_vars {
        cmd.env(key, value);
    }

    let status = cmd.status().await.context("Failed to run alembic")?;

    if !status.success() {
        bail!("alembic upgrade head failed with exit code {}", status);
    }

    output::status("  ✓ Migrations complete");
    Ok(())
}

pub async fn run_cli_setup(
    inputs: &super::prompts::UserInputs,
    env: &super::prereqs::Environment,
    env_path: &Path,
) -> Result<()> {
    let venv_dir = config::venv_dir()?;
    let pipelit_dir = config::pipelit_dir()?;
    let python = venv_dir.join("bin").join("python");
    let env_vars = parse_env_file(env_path)?;

    output::status("  • Running Pipelit setup...");

    let mut cmd = Command::new(&python);
    cmd.args(["-m", "cli", "setup"])
        .arg("--username")
        .arg(&inputs.admin_username)
        .arg("--password")
        .arg(&inputs.admin_password)
        .arg("--sandbox-mode")
        .arg(&env.sandbox_mode)
        .arg("--redis-url")
        .arg(&inputs.redis_url)
        .arg("--platform-base-url")
        .arg(&inputs.platform_base_url)
        .current_dir(pipelit_dir.join("platform"));

    for (key, value) in &env_vars {
        cmd.env(key, value);
    }

    let output_result = cmd
        .output()
        .await
        .context("Failed to run Pipelit CLI setup")?;

    if !output_result.status.success() {
        let stderr = String::from_utf8_lossy(&output_result.stderr);
        bail!("Pipelit setup failed: {}", stderr);
    }

    let stdout =
        String::from_utf8(output_result.stdout).context("Setup output contains invalid UTF-8")?;
    let result: serde_json::Value =
        serde_json::from_str(&stdout).context("Failed to parse setup output")?;

    let username = result["username"].as_str().unwrap_or("unknown");
    output::status(&format!("  ✓ Created admin user '{}'", username));

    Ok(())
}

pub async fn run_apply_fixture(
    inputs: &super::prompts::UserInputs,
    env_path: &Path,
) -> Result<super::config::FixtureOutput> {
    let venv_dir = config::venv_dir()?;
    let pipelit_dir = config::pipelit_dir()?;
    let python = venv_dir.join("bin").join("python");
    let env_vars = parse_env_file(env_path)?;

    output::status("  • Applying default agent fixture...");

    let mut cmd = Command::new(&python);
    cmd.args(["-m", "cli", "apply-fixture", "default-agent"])
        .arg("--provider")
        .arg(&inputs.llm_provider)
        .arg("--model")
        .arg(&inputs.llm_model)
        .arg("--api-key")
        .arg(&inputs.llm_api_key)
        .arg("--base-url")
        .arg(&inputs.llm_base_url)
        .current_dir(pipelit_dir.join("platform"));

    for (key, value) in &env_vars {
        cmd.env(key, value);
    }

    let output_result = cmd.output().await.context("Failed to run apply-fixture")?;

    if !output_result.status.success() {
        let stderr = String::from_utf8_lossy(&output_result.stderr);
        bail!("apply-fixture failed: {}", stderr);
    }

    let stdout =
        String::from_utf8(output_result.stdout).context("Fixture output contains invalid UTF-8")?;
    let result: serde_json::Value =
        serde_json::from_str(&stdout).context("Failed to parse fixture output")?;

    let workflow_slug = result["workflow_slug"]
        .as_str()
        .context("Missing workflow_slug in fixture output")?
        .to_string();
    let trigger_node_id = result["trigger_node_id"]
        .as_str()
        .context("Missing trigger_node_id in fixture output")?
        .to_string();

    output::status(&format!(
        "  ✓ Created workflow '{}' with trigger '{}'",
        workflow_slug, trigger_node_id
    ));

    Ok(super::config::FixtureOutput {
        workflow_slug,
        trigger_node_id,
    })
}

fn parse_env_file(path: &Path) -> Result<Vec<(String, String)>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    let mut vars = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            vars.push((key.trim().to_string(), value.trim().to_string()));
        }
    }

    Ok(vars)
}
