[CmdletBinding()]
param([string]$InstallDir = $PSScriptRoot)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$base = [IO.Path]::GetFullPath($InstallDir)
$layout = $null
$layoutHelper = Join-Path $PSScriptRoot 'scripts/lib/layout.ps1'
if (Test-Path -LiteralPath $layoutHelper) { . $layoutHelper; $layout = Read-Vpn233Layout $base }
elseif (Test-Path -LiteralPath (Join-Path $base '.vpn233-layout')) { throw 'Missing public layout parser.' }
$binaryDir = if ($env:VPN233_BIN_DIR) { [IO.Path]::GetFullPath($env:VPN233_BIN_DIR) } elseif ($layout) { $layout['bin'] } else { Join-Path $base 'bin' }
if (-not (Test-Path (Join-Path $base 'version.txt')) -or -not (Test-Path -LiteralPath $binaryDir -PathType Container)) { throw 'Refusing uninstall: this is not an installed release directory.' }
& (Join-Path $PSScriptRoot 'stop.ps1') -InstallDir $base
foreach ($binary in @('vpn233-provider-server.exe', 'vpn233-subscribe-server.exe', 'vpn233-client.exe')) {
    $target = Join-Path $binaryDir $binary
    if (Test-Path -LiteralPath $target) { Remove-Item -LiteralPath $target -Force }
}
Write-Host 'Application processes and binaries removed. Configuration, data, logs and backups retained.'
