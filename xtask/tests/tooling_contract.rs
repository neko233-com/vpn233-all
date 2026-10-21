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

#[cfg(windows)]
fn native_ps5_entry_command(
    shell: &std::path::Path,
    script: &std::path::Path,
    explicit: Option<&std::path::Path>,
) -> Command {
    const ALLOWED: [&str; 18] = [
        "PATH",
        "HOME",
        "USERPROFILE",
        "LANG",
        "LC_ALL",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "TEMP",
        "TMP",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
        "LOCALAPPDATA",
        "APPDATA",
        "PATHEXT",
        "OS",
        "PROCESSOR_ARCHITECTURE",
        "PROCESSOR_ARCHITEW6432",
    ];
    let mut command = Command::new(shell);
    command.env_clear();
    for (key, value) in std::env::vars_os() {
        if ALLOWED.contains(&key.to_string_lossy().to_ascii_uppercase().as_str()) {
            command.env(key, value);
        }
    }
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
    ]);
    command.arg(script);
    if let Some(explicit) = explicit {
        command.arg("-InstallDir").arg(explicit);
    }
    command
}

#[cfg(windows)]
fn run_native_ps5_entry(mut command: Command) -> std::process::Output {
    match support::process_output(&mut command, None, Duration::from_secs(60)) {
        Ok(output) => output,
        Err(_) => {
            panic!("native PS5 entry failed before a complete outcome; private details withheld")
        }
    }
}

#[cfg(windows)]
#[test]
fn native_ps5_file_entries_resolve_defaults_and_reject_the_old_parameter_default() {
    const BASE: &str = "$base = [IO.Path]::GetFullPath($InstallDir)";
    const PARAMETER: &str = "param([string]$InstallDir = '')";
    const FALLBACK: &str = "if (-not $InstallDir) { $InstallDir = $PSScriptRoot }";
    // Exit immediately after the real advanced-script parameter binding and
    // directory resolution. Actual installed services are covered by the shared
    // archive lifecycle gate; this test never imports ACL or business modules.
    const CAPTURE: &str = r#"
if ($PSVersionTable.PSVersion.Major -ne 5) { exit 73 }
try {
    $entryReport = [IO.Path]::Combine($PSScriptRoot, 'entry-report.txt')
    $entryStream = [IO.File]::Open($entryReport, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $entryBytes = [Text.Encoding]::UTF8.GetBytes($base)
        $entryStream.Write($entryBytes, 0, $entryBytes.Length)
        $entryStream.Flush()
    } finally { $entryStream.Dispose() }
} catch { exit 74 }
exit 0
"#;
    let root = support::root().unwrap();
    let shell = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    assert!(shell.is_file(), "native PS5 executable is required");
    for name in ["start.ps1", "stop.ps1", "uninstall.ps1"] {
        let source = fs::read_to_string(root.join(name)).unwrap();
        assert!(
            source.matches(BASE).count() == 1,
            "entry capture anchor must be unique"
        );
        assert!(
            source.matches(PARAMETER).count() == 1,
            "entry parameter anchor must be unique"
        );
        assert!(
            source.matches(FALLBACK).count() == 1,
            "entry fallback anchor must be unique"
        );
        let captured = source.replace(BASE, &format!("{BASE}\n{CAPTURE}"));
        for mode in ["default", "explicit", "old-default"] {
            let private = tempfile::Builder::new()
                .prefix("vpn233-native-entry-")
                .tempdir()
                .unwrap();
            vpn233_xtask::network::protect_private_directory(private.path()).unwrap();
            let installed = private.path().join("installed prefix with spaces");
            let explicit = private.path().join("explicit prefix with spaces");
            fs::create_dir(&installed).unwrap();
            fs::create_dir(&explicit).unwrap();
            let installed = support::canonical_path(&installed).unwrap();
            let explicit = support::canonical_path(&explicit).unwrap();
            let report = installed.join("entry-report.txt");
            let script = installed.join(name);
            let contents = if mode == "old-default" {
                captured
                    .replace(PARAMETER, "param([string]$InstallDir = $PSScriptRoot)")
                    .replace(FALLBACK, "")
            } else {
                captured.clone()
            };
            fs::write(&script, contents).unwrap();
            assert!(!report.exists(), "entry report must start absent");
            let mut command = native_ps5_entry_command(
                &shell,
                &script,
                (mode == "explicit").then_some(explicit.as_path()),
            );
            command.current_dir(private.path());
            let output = run_native_ps5_entry(command);
            if mode == "old-default" {
                assert!(
                    !output.status.success(),
                    "old native PS5 parameter default must fail"
                );
                assert!(
                    !report.exists(),
                    "failed old entry must not reach the capture marker"
                );
                continue;
            }
            assert!(
                output.status.success(),
                "native PS5 entry failed; private output withheld"
            );
            assert!(
                output.stdout.is_empty() && output.stderr.is_empty(),
                "entry capture must remain silent"
            );
            let value = fs::read_to_string(&report).unwrap();
            let expected = if mode == "explicit" {
                &explicit
            } else {
                &installed
            };
            assert!(
                std::path::Path::new(&value) == expected,
                "native PS5 resolved the wrong directory"
            );
            // The capture itself also fails closed instead of overwriting an
            // existing report, without disclosing its private path on stderr.
            if name == "start.ps1" && mode == "default" {
                let sentinel = b"synthetic existing entry report";
                fs::write(&report, sentinel).unwrap();
                let output = run_native_ps5_entry(native_ps5_entry_command(&shell, &script, None));
                assert!(
                    output.status.code() == Some(74),
                    "existing entry report must fail closed"
                );
                assert!(
                    fs::read(&report).unwrap() == sentinel,
                    "entry report was overwritten"
                );
            }
        }
    }
}

#[test]
fn powershell_bootstrap_isolates_cargo_without_mutating_caller_environment() {
    let root = support::root().unwrap();
    let mut shells = Vec::new();
    #[cfg(windows)]
    shells.push(
        std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
    );
    let pwsh = if cfg!(windows) { "pwsh.exe" } else { "pwsh" };
    if let Some(shell) = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|directory| directory.join(pwsh))
            .find(|path| path.is_file())
    }) {
        shells.push(shell);
    }
    if shells.is_empty() {
        return;
    }
    let private = tempfile::Builder::new()
        .prefix("vpn233-wrapper-environment-")
        .tempdir()
        .unwrap();
    vpn233_xtask::network::protect_private_directory(private.path()).unwrap();
    let bin = private.path().join("native mock tools");
    fs::create_dir(&bin).unwrap();
    let fixture = private.path().join("synthetic-cargo.rs");
    fs::write(&fixture, r#"
use std::{env, fs::OpenOptions, io::Write, path::Path, process};
fn main() {
    let args: Vec<_> = env::args().skip(1).collect();
    let expected = env::var_os("VPN233_SYNTHETIC_CARGO_TARGET").unwrap();
    let arguments_ok = args.len() == 7 && args[..6] == ["build", "--locked", "--quiet", "-p", "vpn233-xtask", "--target-dir"]
        && Path::new(&args[6]) == Path::new(&expected);
    let flags_absent = ["CARGO_ENCODED_RUSTFLAGS", "RUSTFLAGS", "CARGO_BUILD_TARGET"].iter().all(|key| env::var_os(key).is_none());
    let mut report = OpenOptions::new().write(true).create_new(true).open(env::var_os("VPN233_SYNTHETIC_CARGO_REPORT").unwrap()).unwrap();
    write!(report, "{{\"arguments_ok\":{arguments_ok},\"flags_absent\":{flags_absent}}}").unwrap();
    process::exit(env::var("VPN233_SYNTHETIC_CARGO_EXIT").unwrap().parse().unwrap());
}
"#).unwrap();
    let cargo = bin.join(if cfg!(windows) { "cargo.exe" } else { "cargo" });
    let compiled = match support::process_output(
        Command::new("rustc")
            .args([
                "--edition=2021",
                "--crate-name",
                "vpn233_synthetic_cargo",
                "-C",
                "debuginfo=0",
            ])
            .arg(&fixture)
            .arg("-o")
            .arg(&cargo),
        None,
        Duration::from_secs(60),
    ) {
        Ok(output) => output,
        Err(_) => panic!("native Cargo fixture could not compile; private details withheld"),
    };
    assert!(
        compiled.status.success(),
        "native Cargo fixture compilation failed; private output withheld"
    );
    for (index, shell) in shells.into_iter().enumerate() {
        let workspace = private
            .path()
            .join(format!("synthetic workspace with spaces {index}"));
        fs::create_dir_all(workspace.join("scripts")).unwrap();
        let bootstrap = workspace.join(".cache/xtask-bootstrap/debug");
        fs::create_dir_all(&bootstrap).unwrap();
        fs::write(
            bootstrap.join(if cfg!(windows) {
                "vpn233-xtask.exe"
            } else {
                "vpn233-xtask"
            }),
            b"synthetic file; PathOnly must never execute it",
        )
        .unwrap();
        let wrapper = workspace.join("scripts/xtask.ps1");
        fs::copy(root.join("scripts/xtask.ps1"), &wrapper).unwrap();
        let report = workspace.join("controlled-report.json");
        let driver = workspace.join("environment-driver.ps1");
        fs::write(&driver, r#"
param([string]$Wrapper,[string]$Report,[string]$Workspace)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$keys=@('CARGO_ENCODED_RUSTFLAGS','RUSTFLAGS','CARGO_BUILD_TARGET')
$runs=0
$inheritedEmpty=[Environment]::GetEnvironmentVariables('Process').Contains($keys[1]) -and [Environment]::GetEnvironmentVariables('Process')[$keys[1]] -eq ''
if(-not $inheritedEmpty){throw 'native inherited-empty fixture was not established'}
foreach($scenario in @('inherited','missing','mixed','empty','nonempty')){
    foreach($buildExit in @(0,17)){
        if($scenario -ne 'inherited'){
            foreach($key in $keys){if(Test-Path -LiteralPath ('Env:'+$key)){Remove-Item -LiteralPath ('Env:'+$key)}}
            if($scenario -eq 'mixed'){
                Set-Item -LiteralPath ('Env:'+$keys[1]) -Value ''
                Set-Item -LiteralPath ('Env:'+$keys[2]) -Value 'synthetic target'
            }elseif($scenario -eq 'empty'){
                foreach($key in $keys){Set-Item -LiteralPath ('Env:'+$key) -Value ''}
            }elseif($scenario -eq 'nonempty'){
                foreach($key in $keys){Set-Item -LiteralPath ('Env:'+$key) -Value 'synthetic build flags'}
            }
        }
        $before=[Environment]::GetEnvironmentVariables('Process')
        $location=(Get-Location).Path
        $env:VPN233_SYNTHETIC_CARGO_EXIT=[string]$buildExit
        $env:VPN233_SYNTHETIC_CARGO_TARGET=Join-Path $Workspace '.cache/xtask-bootstrap'
        $env:VPN233_SYNTHETIC_CARGO_REPORT=Join-Path $Workspace ('cargo-report-'+$runs+'.json')
        $global:LASTEXITCODE=97
        $caught=$false
        try {
            $path=@(& $Wrapper -PathOnly)
            if($buildExit -eq 0 -and ($path.Count -ne 1 -or -not (Test-Path -LiteralPath $path[0] -PathType Leaf))){throw 'PathOnly result was incomplete'}
        }catch{$caught=$true}
        if($caught -ne ($buildExit -ne 0)){throw 'unexpected wrapper build outcome'}
        if($global:LASTEXITCODE -ne $buildExit){throw 'wrapper lost the native build exit status'}
        $outcome=[IO.File]::ReadAllText($env:VPN233_SYNTHETIC_CARGO_REPORT)|ConvertFrom-Json
        if(-not $outcome.arguments_ok -or -not $outcome.flags_absent){throw 'Cargo arguments or child environment were incorrect'}
        if((Get-Location).Path -ne $location){throw 'wrapper changed caller location'}
        $after=[Environment]::GetEnvironmentVariables('Process')
        foreach($key in $keys){
            if($after.Contains($key) -ne $before.Contains($key)){throw 'environment presence changed'}
            if($after.Contains($key) -and $after[$key] -cne $before[$key]){throw 'environment value changed'}
        }
        $runs++
    }
}
$bytes=[Text.Encoding]::UTF8.GetBytes((@{runs=$runs;inheritedEmptyPreserved=$true;childFlagsAbsent=$true;callerUnchanged=$true;nativeStatusPreserved=$true}|ConvertTo-Json -Compress))
$stream=[IO.File]::Open($Report,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
try{$stream.Write($bytes,0,$bytes.Length)}finally{$stream.Dispose()}
"#).unwrap();
        let mut command = Command::new(shell);
        command.env_clear();
        for (key, value) in std::env::vars_os() {
            if [
                "PATH",
                "HOME",
                "USERPROFILE",
                "LANG",
                "LC_ALL",
                "SYSTEMROOT",
                "WINDIR",
                "COMSPEC",
                "TEMP",
                "TMP",
                "PROGRAMFILES",
                "PROGRAMFILES(X86)",
                "LOCALAPPDATA",
                "APPDATA",
                "PATHEXT",
                "OS",
                "PROCESSOR_ARCHITECTURE",
                "PROCESSOR_ARCHITEW6432",
            ]
            .contains(&key.to_string_lossy().to_ascii_uppercase().as_str())
            {
                command.env(key, value);
            }
        }
        let path = std::env::join_paths(std::iter::once(bin.clone()).chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        )))
        .unwrap();
        command
            .env("PATH", path)
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env("RUSTFLAGS", "")
            .env("CARGO_BUILD_TARGET", "synthetic inherited target")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&driver)
            .arg("-Wrapper")
            .arg(&wrapper)
            .arg("-Report")
            .arg(&report)
            .arg("-Workspace")
            .arg(&workspace)
            .current_dir(private.path());
        let output = match support::process_output(&mut command, None, Duration::from_secs(60)) {
            Ok(output) => output,
            Err(_) => panic!(
                "PowerShell environment regression could not complete; private details withheld"
            ),
        };
        assert!(
            output.status.success(),
            "PowerShell environment isolation regression failed; private output withheld"
        );
        let result: serde_json::Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
        assert!(
            result["runs"] == 10
                && result["inheritedEmptyPreserved"] == true
                && result["childFlagsAbsent"] == true
                && result["callerUnchanged"] == true
                && result["nativeStatusPreserved"] == true
        );
    }
}
