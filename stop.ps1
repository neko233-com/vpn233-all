[CmdletBinding()]
param([string]$InstallDir = '')
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $InstallDir) { $InstallDir = $PSScriptRoot }
$base = [IO.Path]::GetFullPath($InstallDir)
$layout = $null
$layoutHelper = Join-Path $PSScriptRoot 'scripts/lib/layout.ps1'
if (Test-Path -LiteralPath $layoutHelper) { . $layoutHelper; $layout = Read-Vpn233Layout $base }
elseif (Test-Path -LiteralPath (Join-Path $base '.vpn233-layout')) { throw 'Missing public layout parser.' }
$binaryDir = if ($env:VPN233_BIN_DIR) { $env:VPN233_BIN_DIR } elseif ($layout) { $layout['bin'] } elseif (Test-Path (Join-Path $base 'bin')) { Join-Path $base 'bin' } elseif (Test-Path (Join-Path $base 'generated')) { Join-Path $base 'generated' } else { Join-Path $base 'target/release' }
$configDir = if ($env:VPN233_CONFIG_DIR) { $env:VPN233_CONFIG_DIR } elseif ($layout) { $layout['config'] } else { Join-Path $base 'config' }
$services = @{'provider-server' = 'vpn233-provider-server'; 'subscribe-server' = 'vpn233-subscribe-server'; 'vpn233-client' = 'vpn233-client'}
foreach ($name in $services.Keys) {
    $pidFile = Join-Path $base ('runtime/' + $name + '.pid')
    if (-not (Test-Path -LiteralPath $pidFile)) { continue }
    $processId = 0
    if ([int]::TryParse((Get-Content -LiteralPath $pidFile -Raw).Trim(), [ref]$processId)) {
        $running = Get-Process -Id $processId -ErrorAction SilentlyContinue
        $expected = [IO.Path]::GetFullPath((Join-Path $binaryDir ($services[$name] + '.exe')))
        if ($running -and $running.Path -eq $expected) {
            $identityFile = $pidFile + '.identity'
            $owned = $false
            if (Test-Path -LiteralPath $identityFile -PathType Leaf) {
                $owned = (Get-Content -LiteralPath $identityFile -Raw).Trim() -eq [string]$running.StartTime.ToUniversalTime().Ticks
            } else {
                # Legacy PID files need command-line ownership as a second check.
                $config = [IO.Path]::GetFullPath((Join-Path $configDir ($name + '.yaml')))
                $command = Get-CimInstance -ClassName Win32_Process -Filter ('ProcessId=' + $processId) -ErrorAction Stop
                $configPattern = '(?:^|\s)--config\s+(?:"' + [regex]::Escape($config) + '"|' + [regex]::Escape($config) + ')(?=\s|$)'
                $owned = $command -and $command.CommandLine -match $configPattern
            }
            if ($owned) {
                Stop-Process -Id $processId -ErrorAction Stop
                $running.WaitForExit(5000) | Out-Null
                $running.Refresh()
                if (-not $running.HasExited) { throw 'VPN233 process did not stop. Its PID record was retained.' }
            }
        }
    }
    Remove-Item -LiteralPath $pidFile -Force
    if (Test-Path -LiteralPath ($pidFile + '.identity')) { Remove-Item -LiteralPath ($pidFile + '.identity') -Force }
}
Write-Host 'VPN233 processes stopped.'
