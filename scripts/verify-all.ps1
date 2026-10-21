# Local verification. Quick reuses dependencies and omits release builds, E2E
# and network audits; it is never a release sign-off. Missing checks fail closed.
[CmdletBinding()]
param(
    [switch]$Quick,
    [switch]$NoQuickCache,
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
$cacheTicket = ''
$cacheFinished = $false
$cacheEligible = $Quick -and -not ($NoQuickCache -or $SkipRust -or $SkipWeb -or $SkipScripts -or $SkipDocs -or $SkipE2E -or $SkipAudit)
$cacheEngine = (Get-Process -Id $PID).Path
$script:XTask = ''

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
function Invoke-QuickCache {
    param([string[]]$Arguments)
    try {
        $output = & $script:XTask quick-cache @Arguments --flavor powershell --engine $cacheEngine 2>$null
        if ($LASTEXITCODE -eq 0) { return ($output | ConvertFrom-Json) }
    }
    catch { Write-Verbose 'Quick cache unavailable; real checks required' }
    return $null
}

Push-Location $RootDir
try {
    $script:XTask = & (Join-Path $PSScriptRoot 'xtask.ps1') -PathOnly
    $env:VPN233_XTASK_BIN = $script:XTask
    $targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $RootDir 'target' }
    if (-not [IO.Path]::IsPathRooted($targetDir)) { $targetDir = Join-Path $RootDir $targetDir }
    $releaseDir = [IO.Path]::GetFullPath((Join-Path $targetDir 'release'))
    $env:CARGO_ENCODED_RUSTFLAGS = & $script:XTask build-environment
    if ($LASTEXITCODE -ne 0) { throw 'Cannot configure reproducible Rust build environment' }
    Invoke-Step 'version chain and candidate/index privacy' {
        Invoke-Native $script:XTask @('version')
        Invoke-Native $script:XTask @('privacy')
        Invoke-Native git @('diff', '--check')
    }
    if (-not $Quick) {
        Invoke-Step 'full Git history privacy (redacted)' { Invoke-Native $script:XTask @('privacy', '--history') }
        Invoke-Step 'gitleaks full Git history (redacted)' { Invoke-Native gitleaks @('git', '--config', '.gitleaks.toml', '--redact', '--no-banner', '--exit-code', '1') }
    }
    else {
        Invoke-Step 'current full Git history privacy (redacted)' { Invoke-Native $script:XTask @('privacy', '--history-only') }
    }
    if ($cacheEligible -and -not $script:Failures) {
        $cached = Invoke-QuickCache @('probe')
        if ($cached) {
            Write-Host $cached.message
            if ($cached.hit) {
                $watch.Stop()
                Write-Host ('QUICK CACHED (not release validation): live gates passed in ' + [math]::Round($watch.Elapsed.TotalSeconds, 2) + 's')
                $cacheFinished = $true
                exit 0
            }
            $cacheTicket = $cached.ticket
        }
        else { Write-Host 'Quick cache unavailable; real checks required' }
    }
    if (-not $SkipRust) {
        Invoke-Step 'Rust format' { Invoke-Native cargo @('fmt', '--all', '--', '--check') }
        Invoke-Step 'Rust clippy (locked)' { Invoke-Native cargo @('clippy', '--locked', '--workspace', '--all-targets', '--', '-D', 'warnings') }
        Invoke-Step 'Rust tests (locked)' { Invoke-Native cargo @('test', '--locked', '--workspace', '--no-fail-fast') }
        if (-not $Quick) { Invoke-Step 'release binaries' { Invoke-Native cargo @('build', '--locked', '--release', '--workspace', '--exclude', 'vpn233-xtask') } }
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
    }
    if (-not $SkipDocs) { Invoke-Step 'documentation contract' { Invoke-Native (Get-Bash) @('scripts/e2e/check-docs.sh') } }
    if (-not $Quick -and -not $SkipAudit) {
        Invoke-Step 'RustSec dependency audit' { Invoke-Native cargo @('audit') }
        Invoke-Step 'npm dependency audit (high+)' { Invoke-Native npm @('audit', '--prefix', 'admin-web', '--audit-level=high') }
    }
    if (-not $Quick -and -not $SkipE2E) {
        Invoke-Step 'official Mihomo parser: generated inbounds and core fixtures' {
            $fixtureDir = Join-Path ([IO.Path]::GetTempPath()) ('vpn233-core-fixtures-' + [guid]::NewGuid().ToString('N'))
            try {
                Invoke-Native cargo @('run', '--locked', '--quiet', '-p', 'vpn233-core-lib-rust', '--example', 'mihomo_fixtures', '--', $fixtureDir)
                Invoke-Native $script:XTask @('network', 'mihomo', '--download-core', '--cache-dir', '.cache/tools/mihomo', '--provider-bin', (Join-Path $releaseDir 'vpn233-provider-server.exe'), '--core-fixtures-dir', $fixtureDir)
            }
            finally {
                $temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
                $resolvedFixture = [IO.Path]::GetFullPath($fixtureDir)
                if (-not $resolvedFixture.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Unsafe fixture cleanup path' }
                if (Test-Path -LiteralPath $resolvedFixture) { Remove-Item -LiteralPath $resolvedFixture -Recurse -Force }
            }
        }
        Invoke-Step 'Windows E2E (real processes, loopback)' { Invoke-Native powershell @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'scripts/e2e/e2e.ps1', '-BinDir', $releaseDir) }
        Invoke-Step 'real browser UI, sessions, mobile and zero external requests' {
            Invoke-Native npx @('--prefix', 'admin-web', 'playwright', 'install', 'chromium')
            $previousUIBinary = $env:VPN233_UI_BINARY
            try {
                $env:VPN233_UI_BINARY = Join-Path $releaseDir 'vpn233-subscribe-server.exe'
                Invoke-Native npm @('run', 'test:ui', '--prefix', 'admin-web')
            }
            finally { $env:VPN233_UI_BINARY = $previousUIBinary }
        }
        Invoke-Step 'real Windows release packaging/checksums/privacy/docs links' {
            $bundleEnvironment = @{
                PLATFORM = 'windows-x86_64'; TARGET = 'x86_64-pc-windows-msvc'; EXE_SUFFIX = '.exe'
                VERSION = (Get-Content -LiteralPath 'version.txt' -Raw).Trim()
                TARGET_DIR = $releaseDir.Replace('\', '/'); BUNDLE_ROOT = 'target/verify-package/assets'
            }
            $previousBundleEnvironment = @{}
            foreach ($name in $bundleEnvironment.Keys) {
                $previousBundleEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
            }
            try {
                foreach ($name in $bundleEnvironment.Keys) {
                    [Environment]::SetEnvironmentVariable($name, $bundleEnvironment[$name], 'Process')
                }
                Invoke-Native (Get-Bash) @('.github/scripts/assemble-bundle.sh')
            }
            finally {
                foreach ($name in $previousBundleEnvironment.Keys) {
                    [Environment]::SetEnvironmentVariable($name, $previousBundleEnvironment[$name], 'Process')
                }
            }
        }
    }
    if ($cacheTicket -and -not $script:Failures) {
        $saved = Invoke-QuickCache @('store', '--ticket', $cacheTicket, '--checks', [string]$script:Checks, '--elapsed', $watch.Elapsed.TotalSeconds.ToString('F2', [Globalization.CultureInfo]::InvariantCulture))
        if ($saved) { Write-Host $saved.message }
        $cacheFinished = $true
    }
}
finally {
    if ($Quick -and -not $cacheFinished) {
        if ($cacheTicket) { $null = Invoke-QuickCache @('invalidate', '--ticket', $cacheTicket) }
        else { $null = Invoke-QuickCache @('clear') }
    }
    Pop-Location
}
$watch.Stop()
$mode = 'FULL'
if ($Quick) { $mode = 'QUICK (not release validation)' }
if ($SkipRust -or $SkipWeb -or $SkipScripts -or $SkipDocs -or $SkipE2E -or $SkipAudit) { $mode = 'PARTIAL (not release validation)' }
Write-Host ($mode + ': ' + $script:Checks + ' checks, ' + $script:Failures + ' failures, ' + [math]::Round($watch.Elapsed.TotalSeconds, 2) + 's')
if ($script:Failures) { exit 1 }
exit 0
