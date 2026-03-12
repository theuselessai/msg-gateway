use anyhow::Result;
use dialoguer::Confirm;

use super::init::config;
use crate::output;

pub fn run() -> Result<()> {
    let config_dir = config::config_dir()?;
    let data_dir = config::data_dir()?;
    let pipelit_config_dir = dirs::config_dir()
        .map(|d| d.join("pipelit"))
        .unwrap_or_default();

    // Stop running processes first
    let pid_path = data_dir.join("plit.pid");
    if pid_path.exists() {
        output::status("Stopping running plit stack first...");
        let _ = super::stop::run();
    }

    let mut dirs_to_remove: Vec<(&str, std::path::PathBuf)> = Vec::new();

    if config_dir.exists() {
        dirs_to_remove.push(("plit config", config_dir.clone()));
    }
    if data_dir.exists() {
        dirs_to_remove.push(("plit data", data_dir.clone()));
    }
    if pipelit_config_dir.exists() {
        dirs_to_remove.push(("pipelit config", pipelit_config_dir.clone()));
    }

    if dirs_to_remove.is_empty() {
        output::status("Nothing to remove — plit is not installed.");
        return Ok(());
    }

    output::status("This will remove:");
    for (label, path) in &dirs_to_remove {
        output::status(&format!("  {} — {}", label, path.display()));
    }

    let confirm = Confirm::new()
        .with_prompt("Continue?")
        .default(false)
        .interact()?;

    if !confirm {
        output::status("Cancelled.");
        return Ok(());
    }

    for (label, path) in &dirs_to_remove {
        match std::fs::remove_dir_all(path) {
            Ok(()) => output::status(&format!("Removed {} ({})", label, path.display())),
            Err(e) => output::error(&format!("Failed to remove {}: {}", path.display(), e)),
        }
    }

    output::status(
        "Done. Redis data was not touched — run `redis-cli FLUSHALL` if you want a full reset.",
    );

    Ok(())
}
