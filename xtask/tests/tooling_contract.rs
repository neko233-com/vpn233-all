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

#[test]
fn verification_tool_paths_stay_outside_the_product_target_cache() {
    let root = support::root().unwrap();
    let bootstrap = std::path::Path::new(".cache/xtask-bootstrap");
    assert!(!bootstrap.starts_with("target"));
    for relative in [
        "scripts/xtask.sh",
        "scripts/xtask.ps1",
        ".cargo/config.toml",
        ".github/workflows/ci.yml",
        ".github/workflows/release.yml",
        ".github/workflows/docs.yml",
    ] {
        let source = fs::read_to_string(root.join(relative)).unwrap();
        assert!(
            source.contains(".cache/xtask-bootstrap") && !source.contains("target/xtask-bootstrap"),
            "verification tool lifecycle must remain outside recursive product-cache pruning"
        );
    }
    for relative in [".github/workflows/ci.yml", "scripts/verify-all.sh"] {
        assert!(
            fs::read_to_string(root.join(relative))
                .unwrap()
                .contains("cargo test --locked --workspace --no-fail-fast"),
            "workspace verification must collect every test target while preserving failure"
        );
    }
    assert!(
        fs::read_to_string(root.join("scripts/verify-all.ps1"))
            .unwrap()
            .contains("@('test', '--locked', '--workspace', '--no-fail-fast')"),
        "PowerShell verification must collect every test target while preserving failure"
    );
    let docs = fs::read_to_string(root.join(".github/workflows/docs.yml"))
        .unwrap()
        .replace("\r\n", "\n");
    let upload = docs
        .split("name: pages-verification-tool")
        .nth(1)
        .unwrap()
        .split("\n  build:")
        .next()
        .unwrap();
    assert!(
        upload.contains("path: .cache/xtask-bootstrap/debug/vpn233-xtask\n")
            && upload.contains("include-hidden-files: true")
            && upload.contains("if-no-files-found: error"),
        "hidden-directory upload must select exactly the scanned tool and fail if missing"
    );
}
