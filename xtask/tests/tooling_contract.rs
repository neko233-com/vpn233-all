use std::{fs, process::Command, time::Duration};
use vpn233_xtask::support;

#[test]
fn active_repository_tooling_has_no_python_sources_or_invocations() {
    let root = support::root().unwrap();
    let output = support::process_output(
        Command::new("git")
            .args([
                "ls-files",
                "--cached",
                "--others",
                "--exclude-standard",
                "-z",
            ])
            .current_dir(&root),
        None,
        Duration::from_secs(20),
    )
    .unwrap();
    assert!(output.status.success());
    let interpreter = regex::Regex::new(
        r"(?i)(?:^|[\s;&|])(?:python[0-9.]*(?:\.exe)?|py(?:\.exe)?)(?:[\s;&|]|$)|setup-python",
    )
    .unwrap();
    for raw in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        let name = std::str::from_utf8(raw).unwrap();
        let path = root.join(name);
        if !path.is_file() {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        assert!(
            !matches!(extension.as_str(), "py" | "pyw" | "pyc"),
            "interpreter source reintroduced in Git candidates"
        );
        if matches!(extension.as_str(), "sh" | "ps1" | "yml" | "yaml")
            && (name.starts_with("scripts/") || name.starts_with(".github/"))
        {
            let source = fs::read_to_string(path).unwrap();
            assert!(
                !source
                    .lines()
                    .filter(|line| !line.trim_start().starts_with('#'))
                    .any(|line| interpreter.is_match(line)),
                "interpreter invocation reintroduced in active tooling"
            );
        }
    }
}
