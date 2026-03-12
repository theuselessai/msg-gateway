use anyhow::{bail, Context, Result};

use super::init::config;
use crate::output;

pub fn run() -> Result<()> {
    let pid_path = config::data_dir()?.join("plit.pid");

    if !pid_path.exists() {
        bail!(
            "plit is not running (no PID file at {})",
            pid_path.display()
        );
    }

    let content = std::fs::read_to_string(&pid_path)
        .with_context(|| format!("Failed to read {}", pid_path.display()))?;
    let pid: i32 = content
        .trim()
        .parse()
        .with_context(|| format!("Invalid PID in {}", pid_path.display()))?;

    if pid <= 0 {
        let _ = std::fs::remove_file(&pid_path);
        bail!("Invalid PID {} in {}", pid, pid_path.display());
    }

    output::status(&format!("Stopping plit (PID {})...", pid));

    #[cfg(unix)]
    {
        let ret = unsafe { libc::kill(-pid, libc::SIGTERM) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            let _ = std::fs::remove_file(&pid_path);
            match err.raw_os_error() {
                Some(libc::EPERM) => {
                    bail!("Permission denied sending SIGTERM to process group {}", pid);
                }
                Some(libc::ESRCH) => {
                    output::status("Process already exited.");
                    return Ok(());
                }
                _ => {
                    bail!("Failed to stop process group {}: {}", pid, err);
                }
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = pid;
        bail!("plit stop is only supported on Unix systems");
    }

    let _ = std::fs::remove_file(&pid_path);
    output::status("Stopped.");

    Ok(())
}
