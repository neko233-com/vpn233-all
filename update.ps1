# Preserve configuration/data and delegate payload verification to install.ps1.
[CmdletBinding()]
param(
    [switch]$Check, [switch]$Release, [string]$Version = '', [string]$Bundle = '',
    [string]$Sha256 = '', [string]$Prefix = '', [switch]$System,
    [string]$Mirror = '', [switch]$NoStart, [switch]$Yes, [switch]$DryRun
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($Check) {
    if ($DryRun) { Write-Host 'DRY RUN: would check the latest published version.'; return }
    . (Join-Path $PSScriptRoot 'scripts/lib/mirrors.ps1')
    if ($Mirror) { Set-Vpn233MirrorMode $Mirror }
    $temporary = Join-Path ([IO.Path]::GetTempPath()) ('vpn233-check-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $temporary | Out-Null
    try {
        $file = Join-Path $temporary 'version.txt'
        Receive-Vpn233ReleaseAsset 'version.txt' $file '' -Quiet
        $latest = Get-Vpn233Version ([IO.File]::ReadAllText($file).Trim())
        $installed = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'version.txt')).Trim()
        Write-Host ('Installed: ' + $installed + '; latest published: ' + $latest)
    } finally { Remove-Item -LiteralPath $temporary -Recurse -Force }
    return
}
if ($env:VPN233_UPDATE_URLS) { throw 'VPN233_UPDATE_URLS is no longer supported; use -Mirror or VPN233_GH_MIRRORS and verified releases.' }
$parameters = @{} + $PSBoundParameters
[void]$parameters.Remove('Check')
$parameters['Release'] = $true
if (-not $Prefix) { $parameters['Prefix'] = $PSScriptRoot }
if (-not $Bundle -and $env:VPN233_UPDATE_BUNDLE) { $parameters['Bundle'] = $env:VPN233_UPDATE_BUNDLE }
if (-not $Sha256 -and $env:VPN233_UPDATE_SHA256) { $parameters['Sha256'] = $env:VPN233_UPDATE_SHA256 }
$previousBin = $env:VPN233_BIN_DIR
$previousUnsigned = $env:VPN233_ALLOW_UNSIGNED
try {
    if ($env:VPN233_ALLOW_UNSIGNED_UPDATE -eq '1') { $env:VPN233_ALLOW_UNSIGNED = '1' }
    & (Join-Path $PSScriptRoot 'install.ps1') @parameters
} finally { $env:VPN233_BIN_DIR = $previousBin; $env:VPN233_ALLOW_UNSIGNED = $previousUnsigned }
