use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::{path::PathBuf, process::Command as ProcessCommand};
use vpn233_client::{run, Config};

#[derive(Parser, Debug)]
#[command(name = "vpn233-client", version)]
struct Args {
    #[arg(
        short,
        long,
        env = "VPN233_CLIENT_CONFIG",
        default_value = "vpn233-client/config.yaml"
    )]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    Update,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let args = Args::parse();
    if matches!(args.command, Some(Command::Update)) {
        return run_update();
    }
    run(Config::load(&args.config).await?).await
}

fn run_update() -> Result<()> {
    let script = if cfg!(windows) {
        "update.ps1"
    } else {
        "update.sh"
    };
    let path = if let Ok(path) = std::env::var("VPN233_UPDATE_SCRIPT") {
        PathBuf::from(path)
    } else {
        std::env::current_exe()?
            .parent()
            .and_then(|path| path.parent())
            .context("cannot locate installation root")?
            .join(script)
    };
    anyhow::ensure!(path.exists(), "update script not found");
    let status = if cfg!(windows) {
        ProcessCommand::new("powershell")
            .args(["-NoProfile", "-File"])
            .arg(path)
            .status()?
    } else {
        ProcessCommand::new("sh").arg(path).status()?
    };
    anyhow::ensure!(status.success(), "update script failed with {status}");
    Ok(())
}
