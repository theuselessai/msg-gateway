use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use tokio::process::Command;

use super::init::config;
use crate::output;

pub async fn run(dev: bool) -> Result<()> {
    let pid_path = pid_file_path()?;
    if is_running(&pid_path) {
        bail!(
            "plit is already running (PID file: {}). Use `plit stop` first.",
            pid_path.display()
        );
    }

    let config_json = config::config_json_path()?;
    let env_path = config::dot_env_path()?;
    let pipelit_dir = config::pipelit_dir()?;
    let venv_dir = config::venv_dir()?;
    let platform_dir = pipelit_dir.join("platform");

    if !config_json.exists() {
        bail!(
            "Config not found at {}. Run `plit init` first.",
            config_json.display()
        );
    }
    if !platform_dir.exists() {
        bail!(
            "Pipelit not found at {}. Run `plit init` first.",
            platform_dir.display()
        );
    }

    let raw = std::fs::read_to_string(&config_json)
        .with_context(|| format!("Failed to read {}", config_json.display()))?;
    let cfg: serde_json::Value =
        serde_json::from_str(&raw).context("Failed to parse config.json")?;

    let pipelit_port = extract_pipelit_port(&cfg)?;
    let gateway_bin = find_gateway_binary()?;
    let venv_bin = venv_dir.join("bin");
    let honcho_bin = venv_bin.join("honcho");

    if !honcho_bin.exists() {
        bail!(
            "honcho not found at {}. Run `plit init` first.",
            honcho_bin.display()
        );
    }

    let procfile = generate_procfile(&gateway_bin, &config_json, &venv_bin, pipelit_port, dev);
    let procfile_path = config::config_dir()?.join("Procfile");
    std::fs::write(&procfile_path, &procfile)
        .with_context(|| format!("Failed to write {}", procfile_path.display()))?;

    output::status("Starting plit stack...");
    if dev {
        output::status("  (dev mode — frontend dev server included)");
    }

    let mut child = Command::new(&honcho_bin)
        .args(["-f"])
        .arg(&procfile_path)
        .args(["-e"])
        .arg(&env_path)
        .arg("start")
        .current_dir(&platform_dir)
        .spawn()
        .context("Failed to start honcho")?;

    if let Some(pid) = child.id() {
        let _ = std::fs::write(&pid_path, pid.to_string());
    }

    let status = child.wait().await?;

    let _ = std::fs::remove_file(&pid_path);

    if !status.success() {
        if status.code().is_none() {
            return Ok(());
        }
        bail!("plit exited with {}", status);
    }

    Ok(())
}

fn generate_procfile(
    gateway_bin: &std::path::Path,
    config_json: &std::path::Path,
    venv_bin: &std::path::Path,
    pipelit_port: u16,
    dev: bool,
) -> String {
    let gw = gateway_bin.display();
    let cfg = config_json.display();
    let uvicorn = venv_bin.join("uvicorn").display().to_string();
    let rq = venv_bin.join("rq").display().to_string();

    let mut lines = vec![
        format!("gateway: GATEWAY_CONFIG={cfg} {gw}"),
        format!("pipelit: {uvicorn} main:app --host 0.0.0.0 --port {pipelit_port}"),
        format!(
            "scheduler: {rq} worker --worker-class worker_class.PipelitWorker workflows --with-scheduler"
        ),
        format!("worker: {rq} worker-pool workflows -w worker_class.PipelitWorker -n 4"),
    ];

    if dev {
        lines.push("frontend: npm --prefix frontend run dev".to_string());
    }

    lines.join("\n") + "\n"
}

fn find_gateway_binary() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("Could not determine current executable path")?;
    if let Some(dir) = exe.parent() {
        let candidate = dir.join("plit-gw");
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Ok(PathBuf::from("plit-gw"))
}

fn extract_pipelit_port(cfg: &serde_json::Value) -> Result<u16> {
    let url = cfg["backends"]["pipelit"]["inbound_url"]
        .as_str()
        .context("Missing backends.pipelit.inbound_url in config")?;

    let parsed: url::Url = url.parse().context("Invalid pipelit inbound_url")?;
    parsed
        .port()
        .context("No port in pipelit inbound_url — expected http://localhost:PORT/...")
}

fn pid_file_path() -> Result<PathBuf> {
    Ok(config::data_dir()?.join("plit.pid"))
}

fn is_running(pid_path: &std::path::Path) -> bool {
    let Ok(content) = std::fs::read_to_string(pid_path) else {
        return false;
    };
    let Ok(pid) = content.trim().parse::<u32>() else {
        return false;
    };

    #[cfg(unix)]
    {
        let ret = unsafe { libc::kill(pid as i32, 0) };
        if ret == 0 {
            return true;
        }
        let err = std::io::Error::last_os_error();
        err.raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}
