//! Self-update helper: runs the bundled `update.ps1` / `update.sh` script.

use std::process::Command as ProcessCommand;

use anyhow::{Context, Result};

pub(crate) fn run_update() -> Result<()> {
    let script = if cfg!(windows) {
        "update.ps1"
    } else {
        "update.sh"
    };
    if let Ok(path) = std::env::var("VPN233_UPDATE_SCRIPT") {
        let status = if cfg!(windows) {
            ProcessCommand::new("powershell")
                .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &path])
                .status()?
        } else {
            ProcessCommand::new("bash").arg(path).status()?
        };
        anyhow::ensure!(status.success(), "update script failed with {status}");
        return Ok(());
    }
    let executable = std::env::current_exe()?;
    let root = executable
        .parent()
        .and_then(|path| path.parent())
        .context("cannot locate installation root")?;
    let path = root.join(script);
    anyhow::ensure!(path.exists(), "update script not found: {}", path.display());
    let status = if cfg!(windows) {
        ProcessCommand::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(path)
            .status()?
    } else {
        ProcessCommand::new("bash").arg(path).status()?
    };
    anyhow::ensure!(status.success(), "update script failed with {status}");
    Ok(())
}
