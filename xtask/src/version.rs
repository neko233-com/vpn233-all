use anyhow::{bail, Context, Result};
use regex::Regex;
use serde_json::Value;
use std::{collections::BTreeMap, fs, io::Write, ops::Range, path::Path};

const LOCAL_PACKAGES: [&str; 5] = [
    "vpn233-provider-server",
    "vpn233-subscribe-server",
    "vpn233-client",
    "vpn233-core-lib-rust",
    "vpn233-xtask",
];

fn cargo_version_span(text: &str) -> Result<Range<usize>> {
    let section = Regex::new(r"(?m)^\[workspace\.package\][ \t]*(?:#[^\r\n]*)?\r?$")?
        .find(text)
        .ok_or_else(|| anyhow::anyhow!("workspace package section missing"))?;
    let tail = &text[section.end()..];
    let boundary = Regex::new(r"(?m)^\[")?
        .find(tail)
        .map_or(tail.len(), |found| found.start());
    let versions = Regex::new(r#"(?m)^version[ \t]*=[ \t]*"([^"]+)"[ \t]*(?:#[^\r\n]*)?\r?$"#)?;
    let matches = versions
        .captures_iter(&tail[..boundary])
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        bail!("workspace version must be unique");
    }
    let value = matches[0].get(1).context("missing workspace version")?;
    Ok(section.end() + value.start()..section.end() + value.end())
}

pub fn current(root: &Path) -> Result<String> {
    let text = fs::read_to_string(root.join("Cargo.toml"))?;
    Ok(text[cargo_version_span(&text)?].to_owned())
}

fn read_json(root: &Path, name: &str, paths: &[&[&str]]) -> Result<Value> {
    let text = fs::read_to_string(root.join(name))?;
    let parsed = serde_json::from_str(&text)?;
    let mut start = 0;
    skip_space(text.as_bytes(), &mut start);
    for path in paths {
        let span = json_span(text.as_bytes(), start, path)?;
        let _: String = serde_json::from_str(&text[span])?;
    }
    Ok(parsed)
}

pub fn check(root: &Path) -> Result<String> {
    let expected = current(root)?;
    let package = read_json(root, "admin-web/package.json", &[&["version"]])?;
    let lock = read_json(
        root,
        "admin-web/package-lock.json",
        &[&["version"], &["packages", "", "version"]],
    )?;
    let version_file = fs::read_to_string(root.join("version.txt"))?;
    if version_file.trim_start_matches('\u{feff}').trim() != expected
        || package.get("version").and_then(Value::as_str) != Some(expected.as_str())
        || lock.get("version").and_then(Value::as_str) != Some(expected.as_str())
        || lock.pointer("/packages//version").and_then(Value::as_str) != Some(expected.as_str())
    {
        bail!("version chain differs; prepare the shared version through scripts/release");
    }
    let cargo_lock = fs::read_to_string(root.join("Cargo.lock"))?;
    let expression = Regex::new(r#"(?m)^name = "([^"]+)"\r?\nversion = "([^"]+)""#)?;
    for name in LOCAL_PACKAGES {
        let matches = expression
            .captures_iter(&cargo_lock)
            .filter(|capture| &capture[1] == name)
            .collect::<Vec<_>>();
        if matches.len() != 1 || matches[0][2] != expected {
            bail!("workspace lock version differs");
        }
    }
    Ok(expected)
}

fn skip_space(bytes: &[u8], cursor: &mut usize) {
    while bytes.get(*cursor).is_some_and(u8::is_ascii_whitespace) {
        *cursor += 1;
    }
}

fn value_end(bytes: &[u8], start: usize) -> Result<usize> {
    let mut values = serde_json::Deserializer::from_slice(&bytes[start..]).into_iter::<Value>();
    values.next().context("missing JSON value")??;
    Ok(start + values.byte_offset())
}

/// Find a unique object path without reordering unrelated JSON or dependencies.
fn json_span(bytes: &[u8], start: usize, keys: &[&str]) -> Result<Range<usize>> {
    if keys.is_empty() {
        return Ok(start..value_end(bytes, start)?);
    }
    if bytes.get(start) != Some(&b'{') {
        bail!("version JSON parent is not an object");
    }
    let mut cursor = start + 1;
    let mut found = None;
    loop {
        skip_space(bytes, &mut cursor);
        if bytes.get(cursor) == Some(&b'}') {
            break;
        }
        let key_end = value_end(bytes, cursor)?;
        let key: String = serde_json::from_slice(&bytes[cursor..key_end])?;
        cursor = key_end;
        skip_space(bytes, &mut cursor);
        if bytes.get(cursor) != Some(&b':') {
            bail!("JSON object delimiter missing");
        }
        cursor += 1;
        skip_space(bytes, &mut cursor);
        let end = value_end(bytes, cursor)?;
        if key == keys[0] {
            if found.is_some() {
                bail!("ambiguous version JSON path");
            }
            found = Some(json_span(bytes, cursor, &keys[1..])?);
        }
        cursor = end;
        skip_space(bytes, &mut cursor);
        match bytes.get(cursor) {
            Some(b',') => cursor += 1,
            Some(b'}') => break,
            _ => bail!("JSON object delimiter missing"),
        }
    }
    found.context("version JSON path missing")
}

fn patch_json(text: &str, paths: &[&[&str]], version: &str) -> Result<String> {
    let _: Value = serde_json::from_str(text)?;
    let mut start = 0;
    skip_space(text.as_bytes(), &mut start);
    let mut ranges = paths
        .iter()
        .map(|path| json_span(text.as_bytes(), start, path))
        .collect::<Result<Vec<_>>>()?;
    for range in &ranges {
        let _: String = serde_json::from_str(&text[range.clone()])?;
    }
    ranges.sort_by_key(|range| std::cmp::Reverse(range.start));
    let replacement = serde_json::to_string(version)?;
    let mut text = text.to_owned();
    for range in ranges {
        text.replace_range(range, &replacement);
    }
    Ok(text)
}

fn stage_changes<'a>(
    root: &Path,
    changes: &BTreeMap<&'a str, String>,
) -> Result<Vec<(&'a str, tempfile::NamedTempFile)>> {
    let mut staged = Vec::new();
    for (name, text) in changes {
        let path = root.join(name);
        let mut temporary =
            tempfile::NamedTempFile::new_in(path.parent().context("version parent missing")?)?;
        temporary.write_all(text.replace("\r\n", "\n").as_bytes())?;
        temporary.as_file().sync_all()?;
        staged.push((*name, temporary));
    }
    Ok(staged)
}

fn persist_staged(
    root: &Path,
    originals: &BTreeMap<&str, Vec<u8>>,
    staged: Vec<(&str, tempfile::NamedTempFile)>,
) -> Result<()> {
    let committed = (|| -> Result<()> {
        for (name, temporary) in staged {
            temporary
                .persist(root.join(name))
                .map_err(|_| anyhow::anyhow!("version metadata publication failed"))?;
        }
        check(root)?;
        Ok(())
    })();
    if committed.is_err() {
        // Attempt every restoration even if a different file cannot be restored.
        // No matching values or filesystem diagnostics leave the native tool.
        let mut failed = false;
        for (name, original) in originals {
            if fs::write(root.join(name), original).is_err() {
                failed = true;
            }
        }
        if failed {
            bail!("version transaction failed; rollback incomplete, inspect metadata locally");
        }
        bail!("version transaction failed; original bytes restored");
    }
    Ok(())
}

pub fn prepare(root: &Path, version: &str) -> Result<()> {
    if !Regex::new(r"\A(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\z")?
        .is_match(version)
    {
        bail!("version must be MAJOR.MINOR.PATCH");
    }
    // Parse and validate every source before preparing any write.
    let mut cargo = fs::read_to_string(root.join("Cargo.toml"))?;
    let span = cargo_version_span(&cargo)?;
    cargo.replace_range(span, version);
    let package = patch_json(
        &fs::read_to_string(root.join("admin-web/package.json"))?,
        &[&["version"]],
        version,
    )?;
    let npm_lock = patch_json(
        &fs::read_to_string(root.join("admin-web/package-lock.json"))?,
        &[&["version"], &["packages", "", "version"]],
        version,
    )?;
    let lock = fs::read_to_string(root.join("Cargo.lock"))?;
    let expression = Regex::new(r#"(?m)(^name = "([^"]+)"\r?\nversion = ")([^"]+)(")"#)?;
    for name in LOCAL_PACKAGES {
        if expression
            .captures_iter(&lock)
            .filter(|capture| &capture[2] == name)
            .count()
            != 1
        {
            bail!("workspace lock package missing or duplicated");
        }
    }
    let lock = expression
        .replace_all(&lock, |capture: &regex::Captures<'_>| {
            if LOCAL_PACKAGES.contains(&&capture[2]) {
                format!("{}{version}{}", &capture[1], &capture[4])
            } else {
                capture[0].to_owned()
            }
        })
        .into_owned();
    let changes = BTreeMap::from([
        ("Cargo.toml", cargo),
        ("Cargo.lock", lock),
        ("version.txt", format!("{version}\n")),
        ("admin-web/package.json", package),
        ("admin-web/package-lock.json", npm_lock),
    ]);
    let originals = changes
        .keys()
        .map(|name| Ok((*name, fs::read(root.join(name))?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    persist_staged(root, &originals, stage_changes(root, &changes)?)
}

pub fn run(args: &[String]) -> Result<()> {
    let root = crate::support::root()?;
    if args.len() == 2 && args[0] == "--prepare" {
        prepare(&root, &args[1])?;
        println!("prepared locally; review and commit the version change before publishing");
    } else if !args.is_empty() {
        bail!("unsupported version arguments");
    }
    println!("version chain: {} (all metadata consistent)", check(&root)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_patch_keeps_dependency_versions_and_original_order() {
        let original = "{\n  \"packages\": {\"\": {\"name\": \"fixture\", \"version\": \"2.0.2\"}, \"other\": {\"version\":\"2.0.2\"}},\n  \"version\": \"2.0.2\"\n}\n";
        let result = patch_json(
            original,
            &[&["version"], &["packages", "", "version"]],
            "2.1.0",
        )
        .unwrap();
        assert_eq!(result.matches("2.1.0").count(), 2);
        assert_eq!(result.matches("2.0.2").count(), 1);
        assert_eq!(result.replace("2.1.0", "2.0.2"), original);
    }

    #[test]
    fn duplicate_target_fields_are_rejected_before_writes() {
        assert!(patch_json(
            r#"{"version":"1.0.0","version":"2.0.0"}"#,
            &[&["version"]],
            "2.1.0"
        )
        .is_err());
    }

    #[test]
    fn workspace_version_ignores_unrelated_package_sections() {
        let text = "[workspace.package]\nversion = \"2.0.2\"\n[package]\nversion = \"9.0.0\"\n";
        assert_eq!(&text[cargo_version_span(text).unwrap()], "2.0.2");
    }

    fn fixture() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::create_dir(root.join("admin-web")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace.package] # public metadata\nversion = \"1.2.3\" # shared version\n",
        )
        .unwrap();
        fs::write(root.join("version.txt"), "1.2.3\n").unwrap();
        fs::write(
            root.join("admin-web/package.json"),
            "{\"name\":\"fixture\",\"version\":\"1.2.3\"}\n",
        )
        .unwrap();
        fs::write(root.join("admin-web/package-lock.json"), "{\"version\":\"1.2.3\",\"packages\":{\"\":{\"version\":\"1.2.3\"},\"dependency\":{\"version\":\"1.2.3\"}}}\n").unwrap();
        fs::write(
            root.join("Cargo.lock"),
            LOCAL_PACKAGES
                .iter()
                .map(|name| format!("[[package]]\nname = \"{name}\"\nversion = \"1.2.3\"\n\n"))
                .collect::<String>(),
        )
        .unwrap();
        directory
    }

    #[test]
    fn preparation_updates_every_workspace_package_and_leaves_dependencies() {
        let fixture = fixture();
        prepare(fixture.path(), "2.1.0").unwrap();
        assert_eq!(check(fixture.path()).unwrap(), "2.1.0");
        assert_eq!(
            read_json(
                fixture.path(),
                "admin-web/package-lock.json",
                &[&["version"], &["packages", "", "version"]]
            )
            .unwrap()["packages"]["dependency"]["version"],
            "1.2.3"
        );
        assert!(fs::read_to_string(fixture.path().join("Cargo.toml"))
            .unwrap()
            .contains("# shared version"));
    }

    #[test]
    fn malformed_or_ambiguous_json_changes_no_source_of_truth() {
        for broken in ["{", r#"{"version":"1.2.3","version":"9.0.0"}"#] {
            let fixture = fixture();
            fs::write(fixture.path().join("admin-web/package-lock.json"), broken).unwrap();
            let paths = [
                "Cargo.toml",
                "Cargo.lock",
                "version.txt",
                "admin-web/package.json",
                "admin-web/package-lock.json",
            ];
            let before = paths.map(|name| fs::read(fixture.path().join(name)).unwrap());
            assert!(prepare(fixture.path(), "2.1.0").is_err());
            assert_eq!(
                before,
                paths.map(|name| fs::read(fixture.path().join(name)).unwrap())
            );
        }
    }

    #[test]
    fn workspace_lock_drift_is_part_of_the_version_gate() {
        let fixture = fixture();
        let path = fixture.path().join("Cargo.lock");
        fs::write(
            &path,
            fs::read_to_string(&path)
                .unwrap()
                .replacen("1.2.3", "9.0.0", 1),
        )
        .unwrap();
        assert!(check(fixture.path()).is_err());
    }

    #[test]
    fn invalid_release_numbers_are_rejected_without_changes() {
        let fixture = fixture();
        for version in ["v2.1.0", "02.1.0", "2.1", "2.1.0-beta", "2.1.0\n"] {
            assert!(prepare(fixture.path(), version).is_err());
        }
        assert_eq!(check(fixture.path()).unwrap(), "1.2.3");
    }

    const MANAGED: [&str; 5] = [
        "Cargo.lock",
        "Cargo.toml",
        "admin-web/package-lock.json",
        "admin-web/package.json",
        "version.txt",
    ];
    fn originals(root: &Path) -> BTreeMap<&'static str, Vec<u8>> {
        MANAGED
            .into_iter()
            .map(|name| (name, fs::read(root.join(name)).unwrap()))
            .collect()
    }
    fn staged_fixture(root: &Path) -> Vec<(&'static str, tempfile::NamedTempFile)> {
        let changes = MANAGED
            .into_iter()
            .map(|name| {
                (
                    name,
                    fs::read_to_string(root.join(name))
                        .unwrap()
                        .replace("1.2.3", "2.1.0"),
                )
            })
            .collect();
        stage_changes(root, &changes).unwrap()
    }
    fn assert_restored(root: &Path, before: &BTreeMap<&str, Vec<u8>>) {
        for (name, bytes) in before {
            assert_eq!(
                &fs::read(root.join(name)).unwrap(),
                bytes,
                "original metadata bytes changed"
            );
        }
    }

    #[test]
    fn check_rejects_duplicate_version_fields_at_every_target_depth() {
        for (name, text) in [
            (
                "admin-web/package.json",
                r#"{"version":"9.9.9","version":"1.2.3"}"#,
            ),
            (
                "admin-web/package-lock.json",
                r#"{"version":"9.9.9","version":"1.2.3","packages":{"":{"version":"1.2.3"}}}"#,
            ),
            (
                "admin-web/package-lock.json",
                r#"{"version":"1.2.3","packages":{"":{"version":"9.9.9"}},"packages":{"":{"version":"1.2.3"}}}"#,
            ),
            (
                "admin-web/package-lock.json",
                r#"{"version":"1.2.3","packages":{"":{"version":"9.9.9"},"":{"version":"1.2.3"}}}"#,
            ),
            (
                "admin-web/package-lock.json",
                r#"{"version":"1.2.3","packages":{"":{"version":"9.9.9","version":"1.2.3"}}}"#,
            ),
        ] {
            let fixture = fixture();
            fs::write(fixture.path().join(name), text).unwrap();
            let before = originals(fixture.path());
            assert!(check(fixture.path()).is_err());
            assert_restored(fixture.path(), &before);
        }
    }

    #[test]
    fn post_publication_validation_failure_restores_all_original_bytes() {
        use std::io::Seek;
        let fixture = fixture();
        let root = fixture.path();
        let before = originals(root);
        let mut staged = staged_fixture(root);
        let (_, stamp) = staged
            .iter_mut()
            .find(|(name, _)| *name == "version.txt")
            .unwrap();
        stamp.as_file_mut().set_len(0).unwrap();
        stamp.as_file_mut().rewind().unwrap();
        stamp.write_all(b"9.9.9\n").unwrap();
        stamp.as_file().sync_all().unwrap();
        assert!(persist_staged(root, &before, staged).is_err());
        assert_restored(root, &before);
        assert_eq!(check(root).unwrap(), "1.2.3");
    }

    #[cfg(windows)]
    #[test]
    fn real_windows_rename_denial_restores_previous_metadata() {
        use std::os::windows::fs::OpenOptionsExt;
        let fixture = fixture();
        let root = fixture.path();
        let before = originals(root);
        let staged = staged_fixture(root);
        // Permit reads and overwrites, but deny deletion/rename of this actual
        // destination after the earlier version files have been published.
        let locked = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0x1 | 0x2)
            .open(root.join("admin-web/package.json"))
            .unwrap();
        assert!(persist_staged(root, &before, staged).is_err());
        assert_restored(root, &before);
        drop(locked);
        assert_eq!(check(root).unwrap(), "1.2.3");
    }

    #[cfg(unix)]
    #[test]
    fn real_unix_directory_rename_denial_restores_previous_metadata() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let fixture = fixture();
        let root = fixture.path();
        let before = originals(root);
        let staged = staged_fixture(root);
        let parent = root.join("admin-web");
        let original = fs::metadata(&parent).unwrap().permissions();
        struct Restore(std::path::PathBuf, fs::Permissions);
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = fs::set_permissions(&self.0, self.1.clone());
            }
        }
        let restore = Restore(parent.clone(), original);
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).unwrap();
        let result = persist_staged(root, &before, staged);
        drop(restore);
        assert!(result.is_err());
        assert_restored(root, &before);
        assert_eq!(check(root).unwrap(), "1.2.3");
    }

    #[test]
    fn blocked_rollback_still_attempts_every_other_original() {
        let fixture = fixture();
        let root = fixture.path();
        let before = originals(root);
        let staged = staged_fixture(root);
        let blocked = root.join("Cargo.lock");
        fs::remove_file(&blocked).unwrap();
        fs::create_dir(&blocked).unwrap();
        fs::write(blocked.join("unrelated.txt"), b"public blocker").unwrap();
        fs::write(root.join("Cargo.toml"), b"partially updated public fixture").unwrap();
        let error = persist_staged(root, &before, staged).unwrap_err();
        assert!(error.to_string().contains("rollback incomplete"));
        for (name, bytes) in before.iter().filter(|(name, _)| **name != "Cargo.lock") {
            assert_eq!(&fs::read(root.join(name)).unwrap(), bytes);
        }
        assert_eq!(
            fs::read(blocked.join("unrelated.txt")).unwrap(),
            b"public blocker"
        );
    }
}
