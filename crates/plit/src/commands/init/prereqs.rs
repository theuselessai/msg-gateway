//! Prerequisite checking for `plit init`.
//!
//! Verifies that required external tools are available on PATH,
//! detects container environments, and determines sandbox mode.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Result};

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

    let checks = vec![
        check_binary(
            "python3",
            "python3",
            &["--version"],
            "Install Python 3: https://www.python.org/downloads/ or `sudo apt install python3`",
        ),
        check_binary(
            "python3-venv",
            "python3",
            &["-m", "venv", "-h"],
            "Install venv module: `sudo apt install python3-venv`",
        ),
        check_binary(
            "redis-server",
            "redis-server",
            &["--version"],
            "Install Redis: build from source (`make && make install PREFIX=~/.local`), \
             or use DragonflyDB, or `podman run -d -p 6379:6379 redis`",
        ),
        check_binary(
            "git",
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

    let sandbox_mode = if in_container {
        output::status("  → sandbox mode: container");
        "container".to_string()
    } else if cfg!(target_os = "macos") {
        output::status("  ⚠ macOS detected — bubblewrap is not available on macOS");
        output::status("  → sandbox mode: container (code execution is not sandboxed)");
        "container".to_string()
    } else {
        detect_sandbox_mode()?
    };

    Ok(Environment { sandbox_mode })
}

/// Determine sandbox mode when not in a known container.
///
/// Attempts a bwrap test run (`bwrap --ro-bind / / true`) to check if
/// bubblewrap actually works — not just whether the binary exists.
///
/// - Test succeeds → `"bwrap"`
/// - Binary exists but test fails → already inside a sandboxed env
///   (e.g. bwrap with Alpine rootfs, no CAP_SYS_ADMIN) → `"container"`
/// - Binary not found → show install instructions and bail
fn detect_sandbox_mode() -> Result<String> {
    match Command::new("bwrap")
        .args(["--ro-bind", "/", "/", "/bin/true"])
        .output()
    {
        Ok(out) if out.status.success() => {
            output::status("  ✓ bwrap (test run passed)");
            output::status("  → sandbox mode: bwrap");
            Ok("bwrap".to_string())
        }
        Ok(_) => {
            // Binary exists but can't create namespaces — already sandboxed
            output::status("  ✓ already sandboxed (bwrap test failed — lacking privileges)");
            output::status("  → sandbox mode: container");
            Ok("container".to_string())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            output::status("  ✗ bwrap — not found");
            output::status("");
            output::status("Bubblewrap (bwrap) is required for code sandboxing.");
            output::status("Install it and re-run `plit init`:");
            output::status("  • Ubuntu/Debian: sudo apt install bubblewrap");
            output::status("  • Fedora: sudo dnf install bubblewrap");
            output::status("  • Arch: sudo pacman -S bubblewrap");
            output::status("  • From source: https://github.com/containers/bubblewrap");
            bail!("bubblewrap (bwrap) is not installed.");
        }
        Err(e) => {
            bail!("Failed to check bwrap: {e}");
        }
    }
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
    cmd: &str,
    version_args: &[&str],
    install_hint: &'static str,
) -> PrereqResult {
    match Command::new(cmd).args(version_args).output() {
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
