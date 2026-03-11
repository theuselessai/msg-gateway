//! Prerequisite checking for `plit init`.
//!
//! Verifies that required external tools are available on PATH,
//! detects container environments, and determines sandbox mode.

use std::path::Path;
use std::process::Command;

use anyhow::{Result, bail};

use crate::output;

pub struct Environment {
    pub sandbox_mode: String,
}

struct PrereqResult {
    name: &'static str,
    found: bool,
    version: String,
    install_hint: &'static str,
}

pub fn check_all() -> Result<Environment> {
    let container = detect_container();
    let in_container = container.is_some();

    if let Some(ref ctype) = container {
        output::status(&format!("  ✓ container detected ({})", ctype));
    }

    let mut checks = vec![
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

    if !in_container {
        checks.push(check_binary(
            "bwrap",
            &["--version"],
            "Install bubblewrap: `sudo apt install bubblewrap` or build from source",
        ));
    }

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

    let sandbox_mode = if in_container {
        "container".to_string()
    } else {
        "bwrap".to_string()
    };

    Ok(Environment { sandbox_mode })
}

fn detect_container() -> Option<String> {
    if std::env::var("CODESPACES").ok().as_deref() == Some("true") {
        return Some("codespaces".to_string());
    }
    if std::env::var("GITPOD_WORKSPACE_ID").is_ok() {
        return Some("gitpod".to_string());
    }
    if Path::new("/.dockerenv").exists() {
        return Some("docker".to_string());
    }
    if std::env::var("container").ok().as_deref() == Some("podman") {
        return Some("podman".to_string());
    }
    if let Ok(cgroup) = std::fs::read_to_string("/proc/1/cgroup") {
        if cgroup.contains("docker") {
            return Some("docker".to_string());
        }
        if cgroup.contains("kubepods") {
            return Some("kubernetes".to_string());
        }
        if cgroup.contains("containerd") {
            return Some("containerd".to_string());
        }
    }
    None
}

/// Check if a binary is available and capture its version string.
fn check_binary(
    name: &'static str,
    version_args: &[&str],
    install_hint: &'static str,
) -> PrereqResult {
    match Command::new(name).args(version_args).output() {
        Ok(output) if output.status.success() => {
            let raw = if output.stdout.is_empty() {
                String::from_utf8_lossy(&output.stderr).to_string()
            } else {
                String::from_utf8_lossy(&output.stdout).to_string()
            };
            let version = raw.lines().next().unwrap_or("unknown").trim().to_string();
            PrereqResult {
                name,
                found: true,
                version,
                install_hint,
            }
        }
        Ok(_) | Err(_) => PrereqResult {
            name,
            found: false,
            version: String::new(),
            install_hint,
        },
    }
}
