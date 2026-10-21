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

fn report_wrapper_failure(index: usize, path: &std::path::Path, kind: &str, exit: i32) {
    let data = fs::read(path)
        .ok()
        .filter(|bytes| bytes.len() <= 4096)
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    let mut report = serde_json::json!({"engine_index":index,"kind":kind,"driver_exit":exit});
    for field in [
        "runtime_major",
        "runtime_minor",
        "runtime_patch",
        "stage",
        "expected_exit",
        "native_status",
        "failure_code",
    ] {
        let value = data
            .as_ref()
            .and_then(|data| data[field].as_i64())
            .filter(|number| (i32::MIN as i64..=i32::MAX as i64).contains(number))
            .unwrap_or(-1);
        report[field] = value.into();
    }
    let scenario = data
        .as_ref()
        .and_then(|data| data["scenario"].as_str())
        .filter(|value| {
            [
                "entry",
                "inherited",
                "missing",
                "mixed",
                "empty",
                "nonempty",
            ]
            .contains(value)
        })
        .unwrap_or("unavailable");
    report["scenario"] = scenario.into();
    eprintln!("PowerShell wrapper controlled failure: {report}");
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
    let target = args.get(6).and_then(|argument| Path::new(argument).canonicalize().ok());
    let arguments_ok = args.len() == 7 && args[..6] == ["build", "--locked", "--quiet", "-p", "vpn233-xtask", "--target-dir"]
        && target.is_some() && target == Path::new(&expected).canonicalize().ok();
    let flags_absent = ["CARGO_ENCODED_RUSTFLAGS", "RUSTFLAGS", "CARGO_BUILD_TARGET"].iter().all(|key| env::var_os(key).is_none());
    let mut report = OpenOptions::new().write(true).create_new(true).open(env::var_os("VPN233_SYNTHETIC_CARGO_REPORT").unwrap()).unwrap();
    write!(report, "{{\"arguments_ok\":{arguments_ok},\"flags_absent\":{flags_absent}}}").unwrap();
    process::exit(env::var("VPN233_SYNTHETIC_CARGO_EXIT").unwrap().parse().unwrap());
}
"#).unwrap();
    let cargo = bin.join(if cfg!(windows) { "cargo.exe" } else { "cargo" });
    let mut compiler = Command::new("rustc");
    compiler.args([
        "--edition=2021",
        "--crate-name",
        "vpn233_synthetic_cargo",
        "-C",
        "debuginfo=0",
    ]);
    #[cfg(windows)]
    compiler.args(["-C", "target-feature=+crt-static"]);
    compiler.arg(&fixture).arg("-o").arg(&cargo);
    let compiled = match support::process_output(&mut compiler, None, Duration::from_secs(60)) {
        Ok(output) => output,
        Err(_) => panic!("native Cargo fixture could not compile; private details withheld"),
    };
    assert!(
        compiled.status.success(),
        "native Cargo fixture compilation failed; private output withheld"
    );
    #[cfg(windows)]
    {
        let bytes = fs::read(&cargo).unwrap();
        assert!(
            vpn233_xtask::verify_release::pe_imports(&bytes).is_ok(),
            "native Cargo fixture PE is invalid"
        );
        assert!(
            vpn233_xtask::verify_release::windows_binary_rules(&bytes).is_empty(),
            "native Cargo fixture must meet the production Windows runtime rules"
        );
    }
    for expected in [0, 17] {
        let report = private
            .path()
            .join(format!("native-preflight-{expected}.json"));
        let target = private.path().join("native target with spaces");
        fs::create_dir_all(&target).unwrap();
        let mut command = Command::new(&cargo);
        command
            .args([
                "build",
                "--locked",
                "--quiet",
                "-p",
                "vpn233-xtask",
                "--target-dir",
            ])
            .arg(&target)
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_BUILD_TARGET")
            .env("VPN233_SYNTHETIC_CARGO_TARGET", &target)
            .env("VPN233_SYNTHETIC_CARGO_REPORT", &report)
            .env("VPN233_SYNTHETIC_CARGO_EXIT", expected.to_string());
        let output = match support::process_output(&mut command, None, Duration::from_secs(60)) {
            Ok(output) => output,
            Err(_) => panic!(
                "native Cargo fixture preflight could not complete; private details withheld"
            ),
        };
        assert!(
            output.status.code() == Some(expected),
            "native Cargo preflight exit mismatch: expected={expected} actual={}",
            output.status.code().unwrap_or(-1)
        );
        let result: serde_json::Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
        assert!(
            result["arguments_ok"] == true && result["flags_absent"] == true,
            "native Cargo preflight arguments or environment differ"
        );
    }
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
        let diagnostic = workspace.join("controlled-diagnostic.json");
        let driver = workspace.join("environment-driver.ps1");
        fs::write(&driver, r#"
param([string]$Wrapper,[string]$Report,[string]$Workspace,[string]$Diagnostic)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$global:LASTEXITCODE=97
$script:DriverStage=1
$script:DriverScenario='entry'
$script:ExpectedExit=-1
$script:NativeStatus=97
$script:FailureCode=0
function Save-WrapperDiagnostic {
    $v=$PSVersionTable.PSVersion
    $patch=if($v.PSObject.Properties.Name -contains 'Patch'){$v.Patch}else{$v.Build}
    $json='{"runtime_major":'+$v.Major+',"runtime_minor":'+$v.Minor+',"runtime_patch":'+$patch+',"stage":'+$script:DriverStage+',"scenario":"'+$script:DriverScenario+'","expected_exit":'+$script:ExpectedExit+',"native_status":'+$script:NativeStatus+',"failure_code":'+$script:FailureCode+'}'
    [IO.File]::WriteAllText($Diagnostic,$json)
}
function Fail-WrapperCase { param([int]$Code) $script:FailureCode=$Code;throw 'controlled wrapper regression failure' }
Save-WrapperDiagnostic
try {
$keys=@('CARGO_ENCODED_RUSTFLAGS','RUSTFLAGS','CARGO_BUILD_TARGET')
$runs=0
$script:DriverStage=2;Save-WrapperDiagnostic
$inheritedEmpty=[Environment]::GetEnvironmentVariables('Process').Contains($keys[1]) -and [Environment]::GetEnvironmentVariables('Process')[$keys[1]] -eq ''
if(-not $inheritedEmpty){Fail-WrapperCase 2}
foreach($scenario in @('inherited','missing','mixed','empty','nonempty')){
    foreach($buildExit in @(0,17)){
        $script:DriverScenario=$scenario;$script:ExpectedExit=$buildExit;$script:DriverStage=3;Save-WrapperDiagnostic
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
        $script:NativeStatus=97;$script:DriverStage=4;Save-WrapperDiagnostic
        $caught=$false
        try {
            $path=@(& $Wrapper -PathOnly)
        }catch{$caught=$true}
        $script:NativeStatus=$global:LASTEXITCODE;$script:DriverStage=5;Save-WrapperDiagnostic
        if($caught -ne ($buildExit -ne 0)){Fail-WrapperCase 5}
        if($buildExit -eq 0 -and ($path.Count -ne 1 -or -not (Test-Path -LiteralPath $path[0] -PathType Leaf))){Fail-WrapperCase 12}
        $script:DriverStage=6;Save-WrapperDiagnostic
        if($global:LASTEXITCODE -ne $buildExit){Fail-WrapperCase 6}
        $script:DriverStage=7;Save-WrapperDiagnostic
        if(-not [IO.File]::Exists($env:VPN233_SYNTHETIC_CARGO_REPORT)){Fail-WrapperCase 7}
        $outcome=[IO.File]::ReadAllText($env:VPN233_SYNTHETIC_CARGO_REPORT)|ConvertFrom-Json
        $script:DriverStage=8;Save-WrapperDiagnostic
        if(-not $outcome.arguments_ok){Fail-WrapperCase 8}
        if(-not $outcome.flags_absent){Fail-WrapperCase 13}
        $script:DriverStage=9;Save-WrapperDiagnostic
        if((Get-Location).Path -ne $location){Fail-WrapperCase 9}
        $after=[Environment]::GetEnvironmentVariables('Process')
        foreach($key in $keys){
            if($after.Contains($key) -ne $before.Contains($key)){Fail-WrapperCase 10}
            if($after.Contains($key) -and $after[$key] -cne $before[$key]){Fail-WrapperCase 11}
        }
        $runs++
    }
}
$bytes=[Text.Encoding]::UTF8.GetBytes((@{runs=$runs;inheritedEmptyPreserved=$true;childFlagsAbsent=$true;callerUnchanged=$true;nativeStatusPreserved=$true}|ConvertTo-Json -Compress))
$stream=[IO.File]::Open($Report,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
try{$stream.Write($bytes,0,$bytes.Length)}finally{$stream.Dispose()}
$script:DriverStage=10;Save-WrapperDiagnostic
} catch {
    if($script:FailureCode -eq 0){$script:FailureCode=99}
    try{Save-WrapperDiagnostic}catch{}
    exit 1
}
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
        let profile = workspace.join("private profile");
        for (key, directory) in [
            ("HOME", "home"),
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_RUNTIME_DIR", "runtime"),
            ("USERPROFILE", "home"),
            ("APPDATA", "appdata"),
            ("LOCALAPPDATA", "local-appdata"),
        ] {
            let directory = profile.join(directory);
            fs::create_dir_all(&directory).unwrap();
            command.env(key, directory);
        }
        command
            .env("PATH", path)
            .env("POWERSHELL_TELEMETRY_OPTOUT", "1")
            .env("POWERSHELL_UPDATECHECK", "Off")
            .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
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
            .arg("-Diagnostic")
            .arg(&diagnostic)
            .current_dir(private.path());
        let output = match support::process_output(&mut command, None, Duration::from_secs(60)) {
            Ok(output) => output,
            Err(error) => {
                report_wrapper_failure(
                    index,
                    &diagnostic,
                    support::process_failure_kind(&error),
                    -1,
                );
                panic!("PowerShell environment regression could not complete; private details withheld")
            }
        };
        if !output.status.success() {
            report_wrapper_failure(
                index,
                &diagnostic,
                "driver-nonzero",
                output.status.code().unwrap_or(-1),
            );
            panic!("PowerShell environment isolation regression failed; private output withheld");
        }
        let result = fs::read(&report)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        if !result.as_ref().is_some_and(|result| {
            result["runs"] == 10
                && result["inheritedEmptyPreserved"] == true
                && result["childFlagsAbsent"] == true
                && result["callerUnchanged"] == true
                && result["nativeStatusPreserved"] == true
        }) {
            report_wrapper_failure(
                index,
                &diagnostic,
                "report-incomplete",
                output.status.code().unwrap_or(-1),
            );
            panic!(
                "PowerShell environment regression report was incomplete; private details withheld"
            );
        }
    }
}
