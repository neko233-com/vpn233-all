use crate::support;
use anyhow::{bail, Result};
use std::{collections::BTreeSet, env, fs::OpenOptions, io::Write, path::PathBuf};

fn flags_from_environment() -> Vec<String> {
    match env::var("CARGO_ENCODED_RUSTFLAGS") {
        Ok(value) if !value.is_empty() => value.split('\u{1f}').map(str::to_owned).collect(),
        _ => env::var("RUSTFLAGS")
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
    }
}

fn add_mapping(flags: &mut Vec<String>, path: PathBuf, replacement: &str) -> Result<()> {
    let path = support::canonical_path(&path)?;
    let spelling = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("unsupported path encoding"))?;
    let spellings = BTreeSet::from([spelling.to_owned(), spelling.replace('\\', "/")]);
    for spelling in spellings {
        let flag = format!("--remap-path-prefix={spelling}={replacement}");
        if !flags.contains(&flag) {
            flags.push(flag);
        }
    }
    Ok(())
}

pub fn encoded_flags() -> Result<String> {
    let mut flags = flags_from_environment();
    if cfg!(windows)
        && !flags
            .iter()
            .any(|flag| flag == "-Ctarget-feature=+crt-static")
    {
        flags.push("-Ctarget-feature=+crt-static".into());
    }
    let home = env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .ok_or_else(|| anyhow::anyhow!("home directory unavailable"))?;
    add_mapping(&mut flags, PathBuf::from(home), "/user")?;
    add_mapping(&mut flags, support::root()?, "/vpn233")?;
    for (key, replacement) in [("CARGO_HOME", "/cargo"), ("RUSTUP_HOME", "/rustup")] {
        if let Some(value) = env::var_os(key).filter(|value| !value.is_empty()) {
            add_mapping(&mut flags, PathBuf::from(value), replacement)?;
        }
    }
    if flags.iter().any(|flag| flag.contains(['\r', '\n', '\0'])) {
        bail!("build flags contain an invalid environment line");
    }
    Ok(flags.join("\u{1f}"))
}

pub fn run(args: &[String]) -> Result<()> {
    if args.is_empty() {
        println!("{}", encoded_flags()?);
    } else if args == ["--github"] {
        let path = env::var_os("GITHUB_ENV")
            .ok_or_else(|| anyhow::anyhow!("GitHub environment unavailable"))?;
        let mut output = OpenOptions::new().append(true).open(path)?;
        writeln!(output, "CARGO_ENCODED_RUSTFLAGS={}", encoded_flags()?)?;
    } else {
        bail!("unsupported build-environment arguments");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping_keeps_spaces_inside_one_encoded_flag_and_is_idempotent() {
        let parent = tempfile::tempdir().unwrap();
        let path = parent.path().join("synthetic path with spaces");
        std::fs::create_dir(&path).unwrap();
        let mut flags = vec!["-Dwarnings".into()];
        add_mapping(&mut flags, path.clone(), "/fixture").unwrap();
        let first = flags.clone();
        add_mapping(&mut flags, path, "/fixture").unwrap();
        assert_eq!(flags, first);
        assert!(flags
            .iter()
            .any(|flag| flag.ends_with("synthetic path with spaces=/fixture")));
        assert!(flags
            .join("\u{1f}")
            .split('\u{1f}')
            .all(|flag| !flag.is_empty()));
    }
}
