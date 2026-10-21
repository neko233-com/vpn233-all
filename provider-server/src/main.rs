//! Thin binary entry point for vpn233-provider-server.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use vpn233_provider_server::{init_config, run, run_update};

#[derive(Parser, Debug)]
#[command(name = "vpn233-provider-server", version)]
struct Args {
    #[arg(
        short,
        long,
        env = "VPN233_CONFIG",
        default_value = "provider-server/config.yaml"
    )]
    config: PathBuf,
    #[arg(long)]
    init: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the platform update script shipped next to this binary.
    Update,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let args = Args::parse();
    if matches!(args.command, Some(Command::Update)) {
        return run_update();
    }
    if args.init {
        return init_config(&args.config).await;
    }
    run(&args.config).await
}
