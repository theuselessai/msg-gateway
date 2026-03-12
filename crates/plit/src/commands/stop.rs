use anyhow::{Context, Result, bail};

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

    output::status(&format!("Stopping plit (PID {})...", pid));

    #[cfg(unix)]
    {
        let ret = unsafe { libc::kill(-pid, libc::SIGTERM) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            let _ = std::fs::remove_file(&pid_path);
            if err.kind() == std::io::ErrorKind::PermissionDenied {
                bail!("Permission denied sending SIGTERM to process group {}", pid);
            }
            output::status("Process already exited.");
            return Ok(());
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
