use crate::support;
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    env,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
    time::Duration,
};

fn one_argument(args: &[String]) -> Result<&str> {
    if args.len() != 1 {
        bail!("one argument required");
    }
    Ok(&args[0])
}

fn json_input() -> Result<Value> {
    let mut input = Vec::new();
    std::io::stdin()
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut input)?;
    if input.len() > 16 * 1024 * 1024 {
        bail!("JSON input limit exceeded");
    }
    Ok(serde_json::from_slice(&input)?)
}

fn resolved_output(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()?.join(path)
    };
    if absolute
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("parent traversal is forbidden");
    }
    let mut parent = absolute.as_path();
    let mut suffix = Vec::new();
    while !parent.exists() {
        suffix.push(
            parent
                .file_name()
                .context("path parent missing")?
                .to_os_string(),
        );
        parent = parent.parent().context("path parent missing")?;
    }
    let mut resolved = support::canonical_path(parent)?;
    for name in suffix.into_iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

pub fn safe_output(root: &Path, path: &Path) -> Result<PathBuf> {
    let root = support::canonical_path(root)?;
    let resolved = resolved_output(path)?;
    if resolved == root || !resolved.starts_with(root) {
        bail!("output must remain inside the workspace");
    }
    Ok(resolved)
}

pub fn run(command: &str, args: &[String]) -> Result<()> {
    match command {
        "sha256" => println!("{}", support::sha256_file(Path::new(one_argument(args)?))?),
        "checksums" => {
            if args.len() < 2 {
                bail!("checksums needs a directory and archive names");
            }
            let directory = Path::new(&args[0]);
            let mut names = BTreeSet::new();
            let mut manifest = String::new();
            for name in &args[1..] {
                if name.is_empty()
                    || name == "."
                    || name == ".."
                    || !name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
                    || !names.insert(name)
                {
                    bail!("invalid checksum asset name");
                }
            }
            for name in names {
                let path = directory.join(name);
                if !path.is_file() || path.is_symlink() {
                    bail!("checksum input is not a regular archive");
                }
                manifest.push_str(&format!("{}  {name}\n", support::sha256_file(&path)?));
            }
            let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
            temporary.write_all(manifest.as_bytes())?;
            temporary.as_file().sync_all()?;
            temporary.persist(directory.join("SHA256SUMS"))?;
        }
        "candidates" => {
            let extension = one_argument(args)?;
            if !extension.starts_with('.') || extension.contains(['\r', '\n', '\0']) {
                bail!("invalid candidate suffix");
            }
            let output = support::process_output(
                Command::new("git").args([
                    "ls-files",
                    "--cached",
                    "--others",
                    "--exclude-standard",
                    "-z",
                ]),
                None,
                Duration::from_secs(20),
            )?;
            if !output.status.success() {
                bail!("candidate enumeration failed");
            }
            let paths = output
                .stdout
                .split(|byte| *byte == 0)
                .filter(|name| !name.is_empty())
                .map(std::str::from_utf8)
                .collect::<std::result::Result<BTreeSet<_>, _>>()?;
            let mut output = std::io::stdout().lock();
            for name in paths {
                if name.ends_with(extension) && !name.starts_with("legacy/") {
                    output.write_all(name.as_bytes())?;
                    output.write_all(&[0])?;
                }
            }
        }
        "release-dir" => {
            if !args.is_empty() {
                bail!("release-dir takes no arguments");
            }
            let path = env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("target"))
                .join("release");
            let path = if path.is_absolute() {
                path
            } else {
                env::current_dir()?.join(path)
            };
            println!("{}", support::normalize_path(&path).display());
        }
        "json-field" => {
            let pointer = one_argument(args)?;
            let document = json_input()?;
            let value = document.pointer(pointer).context("JSON field absent")?;
            if let Some(value) = value.as_str() {
                println!("{value}");
            } else {
                println!("{value}");
            }
        }
        "cache-lines" => {
            if !args.is_empty() {
                bail!("cache-lines takes no arguments");
            }
            let document = json_input()?;
            let hit = document
                .get("hit")
                .and_then(Value::as_bool)
                .context("cache hit absent")?;
            let ticket = document
                .get("ticket")
                .and_then(Value::as_str)
                .context("cache ticket absent")?;
            let message = document
                .get("message")
                .and_then(Value::as_str)
                .context("cache message absent")?;
            if (!ticket.is_empty()
                && (ticket.len() != 32 || !ticket.bytes().all(|byte| byte.is_ascii_hexdigit())))
                || message.contains(['\n', '\r', '\0'])
            {
                bail!("invalid cache response");
            }
            println!(
                "{} {} {}",
                u8::from(hit),
                if ticket.is_empty() { "-" } else { ticket },
                message
            );
        }
        "safe-output" => {
            safe_output(&support::root()?, Path::new(one_argument(args)?))?;
        }
        _ => bail!("unknown utility"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_output_must_be_strictly_inside_the_root() {
        let temporary = tempfile::tempdir().unwrap();
        assert!(safe_output(temporary.path(), temporary.path()).is_err());
        assert!(safe_output(temporary.path(), &temporary.path().join("future/bundle")).is_ok());
        assert!(safe_output(temporary.path(), &temporary.path().join("../outside")).is_err());
    }

    #[test]
    fn output_does_not_follow_an_external_directory_link() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), root.path().join("linked")).unwrap();
        #[cfg(windows)]
        if std::os::windows::fs::symlink_dir(outside.path(), root.path().join("linked")).is_err() {
            return;
        }
        assert!(safe_output(root.path(), &root.path().join("linked/new")).is_err());
    }
}
