//! Network fixtures and real downloader/parser verification. No Python runtime.
mod e2e_data;
mod echo;
mod mihomo;
mod mirrors;
mod public_install;

use anyhow::{anyhow, bail, Result};
use std::{
    env,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

pub fn run(args: &[String]) -> Result<()> {
    let (command, rest) = args
        .split_first()
        .ok_or_else(|| anyhow!("network subcommand required"))?;
    match command.as_str() {
        "echo" => echo::run(rest),
        "mihomo" => mihomo::run(rest),
        "mirror-tests" => mirrors::run(rest),
        "e2e" => e2e_data::run(rest),
        "public-install" => public_install::run(rest),
        _ => bail!("unknown network subcommand"),
    }
}

fn option(args: &[String], name: &str) -> Result<Option<String>> {
    let positions: Vec<_> = args
        .iter()
        .enumerate()
        .filter(|(_, s)| s.as_str() == name)
        .collect();
    if positions.len() > 1 {
        bail!("duplicate network option");
    }
    positions
        .first()
        .map(|(i, _)| {
            args.get(i + 1)
                .filter(|s| !s.starts_with("--"))
                .cloned()
                .ok_or_else(|| anyhow!("missing network option value"))
        })
        .transpose()
}

fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| anyhow!("private fixture file creation failed"))?;
    file.write_all(bytes)
        .map_err(|_| anyhow!("private fixture file write failed"))?;
    file.flush()?;
    Ok(())
}

fn path_arg(path: &Path) -> String {
    crate::support::normalize_path(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn clean_command(program: &Path) -> Command {
    let mut command = Command::new(crate::support::normalize_path(program));
    for (key, _) in env::vars_os() {
        let upper = key.to_string_lossy().to_ascii_uppercase();
        if upper.starts_with("VPN233_")
            || [
                "GH_TOKEN",
                "GITHUB_TOKEN",
                "GH_ENTERPRISE_TOKEN",
                "GITHUB_ENTERPRISE_TOKEN",
                "BASH_ENV",
                "ENV",
                "HTTP_PROXY",
                "HTTPS_PROXY",
                "ALL_PROXY",
                "NO_PROXY",
            ]
            .contains(&upper.as_str())
        {
            command.env_remove(key);
        }
    }
    command
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env("no_proxy", "127.0.0.1,localhost");
    command
}

fn program(names: &[&str]) -> Option<PathBuf> {
    #[cfg(windows)]
    if names.contains(&"bash") {
        for suffix in ["Git/bin/bash.exe", "Git/usr/bin/bash.exe"] {
            let candidate = PathBuf::from(
                env::var_os("ProgramFiles").unwrap_or_else(|| "C:/Program Files".into()),
            )
            .join(suffix);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    for name in names {
        for directory in env::split_paths(&env::var_os("PATH").unwrap_or_default()) {
            for suffix in if cfg!(windows) {
                vec![".exe", ".EXE", ""]
            } else {
                vec![""]
            } {
                let candidate = directory.join(format!("{name}{suffix}"));
                if candidate.is_file() {
                    if cfg!(windows)
                        && *name == "bash"
                        && candidate
                            .to_string_lossy()
                            .to_ascii_lowercase()
                            .contains("system32")
                    {
                        continue;
                    }
                    return Some(candidate);
                }
            }
        }
    }
    None
}

struct ChildGuard(std::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_file_refuses_existing_file_and_directory() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("ready");
        private_write(&file, b"retained").unwrap();
        assert!(private_write(&file, b"replace").is_err());
        assert!(private_write(temp.path(), b"replace").is_err());
        assert_eq!(std::fs::read(file).unwrap(), b"retained");
    }
}
