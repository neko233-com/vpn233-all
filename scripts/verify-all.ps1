# Local verification. Quick reuses dependencies and omits release builds, E2E
# and network audits; it is never a release sign-off. Missing checks fail closed.
[CmdletBinding()]
param(
    [switch]$Quick,
    [switch]$SkipRust,
    [switch]$SkipWeb,
    [switch]$SkipScripts,
    [switch]$SkipDocs,
    [switch]$SkipE2E,
    [switch]$SkipAudit
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$RootDir = Split-Path -Parent $PSScriptRoot
$script:Failures = 0
$script:Checks = 0
$watch = [Diagnostics.Stopwatch]::StartNew()

function Invoke-Native {
    param([string]$Tool, [string[]]$Arguments = @())
    & $Tool @Arguments | Out-Host
    if ($LASTEXITCODE -ne 0) { throw ($Tool + ' exited with code ' + $LASTEXITCODE) }
}
function Invoke-Step {
    param([string]$Name, [scriptblock]$Action)
    Write-Host ('==> ' + $Name)
    $script:Checks++
    try { & $Action | Out-Host }
    catch { $script:Failures++; Write-Host ('FAILED: ' + $Name + ': ' + $_.Exception.Message) -ForegroundColor Red }
}
function Require-Tool {
    param([string]$Name)
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) { throw ('Required tool missing: ' + $Name) }
}
function Get-Bash {
    foreach ($candidate in @('C:\Program Files\Git\bin\bash.exe', 'C:\Program Files (x86)\Git\bin\bash.exe')) {
        if (Test-Path -LiteralPath $candidate) { return $candidate }
    }
    Require-Tool 'bash'
    return (Get-Command bash).Source
}
function Get-Candidates {
    param([string]$Extension)
    $paths = @(git ls-files --cached --others --exclude-standard)
    if ($LASTEXITCODE -ne 0) { throw 'Cannot enumerate Git candidates' }
    return @($paths | Sort-Object -Unique | Where-Object { $_ -notlike 'legacy/*' -and $_.EndsWith($Extension) -and (Test-Path -LiteralPath $_) })
}

Push-Location $RootDir
try {
    $env:CARGO_ENCODED_RUSTFLAGS = python scripts/build-environment.py
    if ($LASTEXITCODE -ne 0) { throw 'Cannot configure reproducible Rust build environment' }
    Invoke-Step 'version chain and candidate/index privacy' {
        Invoke-Native python @('scripts/release-version.py')
        Invoke-Native python @('scripts/check-privacy.py')
        Invoke-Native git @('diff', '--check')
    }
    if (-not $Quick) {
        Invoke-Step 'full Git history privacy (redacted)' { Invoke-Native python @('scripts/check-privacy.py', '--history') }
        Invoke-Step 'gitleaks full Git history (redacted)' { Invoke-Native gitleaks @('git', '--config', '.gitleaks.toml', '--redact', '--no-banner', '--exit-code', '1') }
    }
    if (-not $SkipRust) {
        Invoke-Step 'Rust format' { Invoke-Native cargo @('fmt', '--all', '--', '--check') }
        Invoke-Step 'Rust clippy (locked)' { Invoke-Native cargo @('clippy', '--locked', '--workspace', '--all-targets', '--', '-D', 'warnings') }
        Invoke-Step 'Rust tests (locked)' { Invoke-Native cargo @('test', '--locked', '--workspace') }
        if (-not $Quick) { Invoke-Step 'release binaries' { Invoke-Native cargo @('build', '--locked', '--release', '--workspace') } }
    }
    if (-not $SkipWeb) {
        Invoke-Step 'admin-web dependencies/build/output gate' {
            $lockHash = (Get-FileHash 'admin-web/package-lock.json' -Algorithm SHA256).Hash.ToLowerInvariant()
            $marker = 'admin-web/node_modules/.vpn233-lock.sha256'
            $dependenciesMatch = (Test-Path -LiteralPath $marker) -and ((Get-Content -LiteralPath $marker -Raw).Trim() -eq $lockHash)
            if (-not $Quick -or -not $dependenciesMatch) {
                Invoke-Native npm @('ci', '--prefix', 'admin-web', '--prefer-offline', '--no-audit')
                Set-Content -LiteralPath $marker -Value $lockHash -Encoding ASCII
            }
            Invoke-Native npm @('run', 'build', '--prefix', 'admin-web')
            Invoke-Native npm @('run', 'check', '--prefix', 'admin-web')
        }
    }
    if (-not $SkipScripts) {
        Invoke-Step 'actionlint with shellcheck' { Require-Tool 'shellcheck'; Invoke-Native actionlint @('-color') }
        Invoke-Step 'shell syntax, shellcheck and installer tests' {
            $bash = Get-Bash
            Require-Tool 'shellcheck'
            foreach ($path in (Get-Candidates '.sh')) {
                Invoke-Native $bash @('-n', $path)
                Invoke-Native shellcheck @('-S', 'warning', $path)
            }
            foreach ($path in @('scripts/tests/test-mirrors.sh', 'scripts/tests/test-installers.sh', 'scripts/tests/test-services.sh')) {
                if (-not (Test-Path $path)) { throw ('Required test missing: ' + $path) }
                Invoke-Native $bash @($path)
            }
        }
        Invoke-Step 'PowerShell parse and PSScriptAnalyzer' {
            Import-Module PSScriptAnalyzer -ErrorAction Stop
            foreach ($path in (Get-Candidates '.ps1')) {
                $tokens = $null
                $parseErrors = $null
                $null = [Management.Automation.Language.Parser]::ParseFile((Join-Path $RootDir $path), [ref]$tokens, [ref]$parseErrors)
                if ($parseErrors.Count) { throw ('PowerShell parse failed: ' + $path) }
                $findings = @(Invoke-ScriptAnalyzer -Path $path -Settings '.github/psscriptanalyzer/PSScriptAnalyzerSettings.psd1')
                if ($findings.Count) { $findings | Format-Table -AutoSize | Out-Host; throw ('PSScriptAnalyzer failed: ' + $path) }
            }
        }
        Invoke-Step 'PowerShell mirror, installer and service ownership tests' {
            foreach ($path in @('scripts/tests/test-mirrors.ps1', 'scripts/tests/test-installers.ps1', 'scripts/tests/test-services.ps1')) {
                Invoke-Native powershell @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $path)
            }
        }
        Invoke-Step 'release and privacy regression tests' { Invoke-Native python @('-m', 'unittest', 'discover', '-s', 'scripts/tests', '-p', 'test_*.py', '-v') }
    }
    if (-not $SkipDocs) { Invoke-Step 'documentation contract' { Invoke-Native (Get-Bash) @('scripts/e2e/check-docs.sh') } }
    if (-not $Quick -and -not $SkipAudit) {
        Invoke-Step 'RustSec dependency audit' { Invoke-Native cargo @('audit') }
        Invoke-Step 'npm dependency audit (high+)' { Invoke-Native npm @('audit', '--prefix', 'admin-web', '--audit-level=high') }
    }
    if (-not $Quick -and -not $SkipE2E) {
        Invoke-Step 'Windows E2E (real processes, loopback)' { Invoke-Native powershell @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'scripts/e2e/e2e.ps1') }
        Invoke-Step 'real browser UI, sessions, mobile and zero external requests' {
            Invoke-Native npx @('--prefix', 'admin-web', 'playwright', 'install', 'chromium')
            Invoke-Native npm @('run', 'test:ui', '--prefix', 'admin-web')
        }
        Invoke-Step 'real Windows release packaging/checksums/privacy/docs links' {
            Invoke-Native (Get-Bash) @('-c', 'PLATFORM=windows-x86_64 TARGET=x86_64-pc-windows-msvc EXE_SUFFIX=.exe VERSION="$(tr -d ''\r\n'' < version.txt)" TARGET_DIR=target/release BUNDLE_ROOT=target/verify-package bash .github/scripts/assemble-bundle.sh')
        }
    }
}
finally { Pop-Location }
$watch.Stop()
$mode = 'FULL'
if ($Quick) { $mode = 'QUICK (not release validation)' }
elseif ($SkipRust -or $SkipWeb -or $SkipScripts -or $SkipDocs -or $SkipE2E -or $SkipAudit) { $mode = 'PARTIAL (not release validation)' }
Write-Host ($mode + ': ' + $script:Checks + ' checks, ' + $script:Failures + ' failures, ' + [math]::Round($watch.Elapsed.TotalSeconds, 2) + 's')
if ($script:Failures) { exit 1 }
exit 0
