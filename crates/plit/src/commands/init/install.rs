//! Pipelit installation for `plit init`.
//!
//! Handles cloning the Pipelit repository, creating a Python virtualenv,
//! installing dependencies, and running database migrations.

use std::path::Path;

use anyhow::{Context, Result, bail};
use tokio::process::Command;

use super::config;
use crate::output;

/// Clone the Pipelit repository if it doesn't already exist.
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

/// Create a Python virtualenv if it doesn't already exist.
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

/// Install Python dependencies from Pipelit's requirements.txt.
pub async fn install_deps() -> Result<()> {
    let venv_dir = config::venv_dir()?;
    let pipelit_dir = config::pipelit_dir()?;
    let pip = venv_dir.join("bin").join("pip");
    let requirements = pipelit_dir.join("requirements.txt");

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

/// Run Alembic database migrations.
/// The .env file must already be written before calling this.
pub async fn run_migrations(env_path: &Path) -> Result<()> {
    let venv_dir = config::venv_dir()?;
    let pipelit_dir = config::pipelit_dir()?;
    let alembic = venv_dir.join("bin").join("alembic");

    if !alembic.exists() {
        output::status("  • Alembic not found in venv, skipping migrations");
        return Ok(());
    }

    output::status("  • Running database migrations...");

    // Read .env file to pass DATABASE_URL and other vars to alembic
    let env_vars = parse_env_file(env_path)?;

    let mut cmd = Command::new(&alembic);
    cmd.args(["upgrade", "head"]).current_dir(&pipelit_dir);

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

/// Start Pipelit temporarily, create the admin user, then kill it.
pub async fn create_admin_user(
    pipelit_port: u16,
    username: &str,
    password: &str,
    env_path: &Path,
) -> Result<()> {
    let venv_dir = config::venv_dir()?;
    let pipelit_dir = config::pipelit_dir()?;
    let python = venv_dir.join("bin").join("python");

    output::status("  • Starting Pipelit temporarily to create admin user...");

    let env_vars = parse_env_file(env_path)?;

    let mut child = Command::new(&python)
        .args([
            "-m",
            "uvicorn",
            "platform.main:app",
            "--port",
            &pipelit_port.to_string(),
            "--host",
            "127.0.0.1",
        ])
        .current_dir(&pipelit_dir)
        .envs(env_vars)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("Failed to start Pipelit")?;

    // Wait for Pipelit to become healthy (up to 30s)
    let base_url = format!("http://localhost:{}", pipelit_port);
    let client = reqwest::Client::new();
    let healthy = wait_for_healthy(&client, &base_url, 30).await;

    if !healthy {
        child.kill().await.ok();
        bail!("Pipelit did not become healthy within 30 seconds");
    }

    // Create admin user via setup endpoint
    let setup_url = format!("{}/api/v1/auth/setup/", base_url);
    let body = serde_json::json!({
        "username": username,
        "password": password,
    });

    let resp = client
        .post(&setup_url)
        .json(&body)
        .send()
        .await
        .context("Failed to call Pipelit setup endpoint")?;

    if resp.status().is_success() {
        output::status(&format!("  ✓ Created admin user '{}'", username));
    } else {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        if body_text.contains("already exists") || body_text.contains("already set up") {
            output::status(&format!(
                "  ⚠ Admin user may already exist ({}), continuing",
                status
            ));
        } else {
            child.kill().await.ok();
            bail!("Failed to create admin user: {} — {}", status, body_text);
        }
    }

    // Kill the temporary Pipelit process
    child.kill().await.ok();
    output::status("  ✓ Stopped temporary Pipelit instance");

    Ok(())
}

/// Poll the health endpoint until it returns 200 or timeout.
async fn wait_for_healthy(client: &reqwest::Client, base_url: &str, timeout_secs: u64) -> bool {
    let health_url = format!("{}/health", base_url);
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(timeout_secs);

    while tokio::time::Instant::now() < deadline {
        if let Ok(resp) = client.get(&health_url).send().await
            && resp.status().is_success()
        {
            return true;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    false
}

/// Parse a simple .env file into key-value pairs.
/// Skips comments and empty lines.
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
