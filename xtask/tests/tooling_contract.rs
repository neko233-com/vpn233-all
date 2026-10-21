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

#[cfg(windows)]
#[test]
fn copied_bootstrap_restores_tls_when_a_locked_tempfile_cannot_be_removed() {
    let root = support::root().unwrap();
    let readme = fs::read_to_string(root.join("README.md")).unwrap();
    let copied = readme
        .split("```powershell")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap()
        .trim();
    let script = r#"
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$candidate=@'
__COPIED_BOOTSTRAP__
'@
$initialTls=[Net.ServicePointManager]::SecurityProtocol
function Invoke-WebRequest {
    [CmdletBinding()]
    param([Parameter(Position=0)][string]$Uri,[switch]$UseBasicParsing,[string]$OutFile,[int]$TimeoutSec)
    if(-not $UseBasicParsing){throw 'bootstrap basic parsing missing'}
    $global:downloads++
    $global:downloadedPath=$OutFile
    $global:lockedFile=[IO.File]::Open($OutFile,[IO.FileMode]::CreateNew,[IO.FileAccess]::ReadWrite,[IO.FileShare]::Read)
    $bytes=[Text.Encoding]::UTF8.GetBytes('synthetic bytes; never execute this file')
    $global:lockedFile.Write($bytes,0,$bytes.Length)
    $global:lockedFile.Flush()
}
function powershell {
    $global:children++
    & $env:ComSpec /d /c "exit $global:fixtureExit"
    $global:LASTEXITCODE=$LASTEXITCODE
}
try {
    foreach($fixtureExit in @(0,17)) {
        $global:fixtureExit=$fixtureExit
        $global:LASTEXITCODE=-99
        $global:downloads=0
        $global:children=0
        $global:downloadedPath=''
        $global:lockedFile=$null
        [Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::SystemDefault
        $failed=$false
        $cleanupFailed=$false
        try {
            try { & ([ScriptBlock]::Create($candidate)) }
            catch {
                $failed=$true
                $cleanupFailed=$_.InvocationInfo.MyCommand.Name -eq 'Remove-Item'
            }
            if(-not $failed -or -not $cleanupFailed){throw 'locked bootstrap must preserve cleanup failure'}
            if($global:downloads -ne 1 -or $global:children -ne 1){throw 'unexpected bootstrap control flow'}
            if($global:LASTEXITCODE -ne $fixtureExit){throw 'bootstrap native status changed'}
            if([Net.ServicePointManager]::SecurityProtocol -ne [Net.SecurityProtocolType]::SystemDefault){throw 'bootstrap TLS was not restored'}
            if(-not [IO.File]::Exists($global:downloadedPath)){throw 'locked bootstrap fixture was unexpectedly removed'}
        }
        finally {
            if($null -ne $global:lockedFile){$global:lockedFile.Dispose()}
            if($global:downloadedPath){[IO.File]::Delete($global:downloadedPath)}
        }
    }
}
finally { [Net.ServicePointManager]::SecurityProtocol=$initialTls }
Write-Output 'locked bootstrap cleanup: 2 scenarios passed; zero network or downloaded-script execution'
"#
    .replace("__COPIED_BOOTSTRAP__", copied);
    // Close the writer before PowerShell opens the script with read sharing.
    let directory = tempfile::tempdir().unwrap();
    let probe = directory.path().join("locked-bootstrap.ps1");
    fs::write(&probe, script).unwrap();
    let powershell = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let output = support::process_output(
        Command::new(powershell)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&probe),
        None,
        Duration::from_secs(30),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "locked tempfile regression failed; private child output withheld"
    );
    assert!(String::from_utf8_lossy(&output.stdout)
        .contains("locked bootstrap cleanup: 2 scenarios passed"));
}
