//! Prerequisite checking for `plit init`.
//!
//! Verifies that required external tools (python3, pip3, redis-server, git)
//! are available on PATH and captures their versions.

use std::process::Command;

use anyhow::{Result, bail};

use crate::output;

/// Result of a single prerequisite check.
struct PrereqResult {
    name: &'static str,
    found: bool,
    version: String,
    install_hint: &'static str,
}

/// Check all prerequisites. Returns `Ok(())` if all are present, or an error
/// with actionable instructions for missing tools.
pub fn check_all() -> Result<()> {
    let checks = vec![
        check_binary(
            "python3",
            &["--version"],
            "Install Python 3: https://www.python.org/downloads/ or `sudo apt install python3`",
        ),
        check_binary(
            "pip3",
            &["--version"],
            "Install pip3: `sudo apt install python3-pip` or `python3 -m ensurepip`",
        ),
        check_binary(
            "redis-server",
            &["--version"],
            "Install Redis: build from source (`make && make install PREFIX=~/.local`), \
             or use DragonflyDB, or `podman run -d -p 6379:6379 redis`",
        ),
        check_binary(
            "git",
            &["--version"],
            "Install git: https://git-scm.com/downloads or `sudo apt install git`",
        ),
    ];

    let mut any_missing = false;

    for result in &checks {
        if result.found {
            output::status(&format!("  ✓ {} ({})", result.name, result.version));
        } else {
            output::status(&format!("  ✗ {} — not found", result.name));
            any_missing = true;
        }
    }

    if any_missing {
        output::status("");
        output::status("Missing prerequisites. Install instructions:");
        for result in &checks {
            if !result.found {
                output::status(&format!("  • {}: {}", result.name, result.install_hint));
            }
        }
        bail!("Missing required prerequisites. Install them and re-run `plit init`.");
    }

    Ok(())
}

/// Check if a binary is available and capture its version string.
fn check_binary(
    name: &'static str,
    version_args: &[&str],
    install_hint: &'static str,
) -> PrereqResult {
    match Command::new(name).args(version_args).output() {
        Ok(output) => {
            let raw = if output.stdout.is_empty() {
                String::from_utf8_lossy(&output.stderr).to_string()
            } else {
                String::from_utf8_lossy(&output.stdout).to_string()
            };
            // Extract first line and trim
            let version = raw.lines().next().unwrap_or("unknown").trim().to_string();
            PrereqResult {
                name,
                found: true,
                version,
                install_hint,
            }
        }
        Err(_) => PrereqResult {
            name,
            found: false,
            version: String::new(),
            install_hint,
        },
    }
}
