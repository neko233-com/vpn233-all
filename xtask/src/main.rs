use std::process::ExitCode;

fn main() -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    let command = arguments.next().unwrap_or_else(|| "help".into());
    let args = arguments.collect::<Vec<_>>();
    let result = match command.as_str() {
        "archive" => vpn233_xtask::archives::run(&args),
        "build-environment" => vpn233_xtask::build_environment::run(&args),
        "network" => vpn233_xtask::network::run(&args),
        "pages-gate" => vpn233_xtask::pages_gate::run(&args),
        "privacy" => vpn233_xtask::privacy::run(&args),
        "quick-cache" => vpn233_xtask::quick_cache::run(&args),
        "verify-release" => vpn233_xtask::verify_release::run(&args),
        "version" => vpn233_xtask::version::run(&args),
        "sha256" | "checksums" | "candidates" | "release-dir" | "json-field" | "cache-lines"
        | "safe-output" => vpn233_xtask::utilities::run(&command, &args),
        "help" | "--help" | "-h" => {
            println!("VPN233 Rust development tools\nCommands: version, privacy, archive, verify-release, build-environment, quick-cache, network, pages-gate\nRun through scripts/xtask.sh or scripts/xtask.ps1; product bundles do not require this tool.");
            Ok(())
        }
        _ => Err(anyhow::anyhow!("unknown command")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => {
            // Never print raw filesystem, subprocess, network or secret values.
            let label = match command.as_str() {
                "archive" | "build-environment" | "network" | "pages-gate" | "privacy"
                | "quick-cache" | "verify-release" | "version" | "sha256" | "checksums"
                | "candidates" | "release-dir" | "json-field" | "cache-lines" | "safe-output" => {
                    command.as_str()
                }
                _ => "command",
            };
            eprintln!("xtask: {label} failed (private details redacted)");
            ExitCode::FAILURE
        }
    }
}
