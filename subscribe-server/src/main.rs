//! Thin CLI entry point: tracing setup, argument parsing, and command dispatch.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "vpn233-subscribe-server", version)]
struct Args {
    #[arg(
        short,
        long,
        env = "VPN233_SUBSCRIBE_CONFIG",
        default_value = "subscribe-server/config.yaml"
    )]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Replace the installed binary with the latest release.
    Update,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let args = Args::parse();
    if matches!(args.command, Some(Command::Update)) {
        return vpn233_subscribe_server::update_self().await;
    }
    vpn233_subscribe_server::run(args.config).await
}
