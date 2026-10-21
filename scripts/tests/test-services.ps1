# Offline process ownership checks: both process lookup and stop are mocked.
[Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidOverwritingBuiltInCmdlets', '', Justification = 'Offline process mocks prevent any real process from being stopped.')]
[Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSReviewUnusedParameter', '', Justification = 'Mock signatures accept the production command parameters.')]
[Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidGlobalVars', '', Justification = 'Mocks called from another script share isolated fixture state, removed in finally.')]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$work = Join-Path ([IO.Path]::GetTempPath()) ('vpn233-services-test-' + [guid]::NewGuid().ToString('N'))
$previousBin = $env:VPN233_BIN_DIR
$previousConfig = $env:VPN233_CONFIG_DIR
$global:Vpn233RuntimeTestState = @{ Stopped = 0; Process = $null; Command = ''; Started = @() }
$script:passed = 0
function Assert-True {
    param([string]$Name, [bool]$Condition)
    if (-not $Condition) { throw ('FAIL: ' + $Name) }
    $script:passed++; Write-Host ('PASS: ' + $Name)
}
function Get-Process {
    param([int]$Id, [string]$ErrorAction)
    if ($Id -ne 12345) { return $null }
    return $global:Vpn233RuntimeTestState.Process
}
function Stop-Process {
    param([int]$Id, [string]$ErrorAction)
    if ($Id -ne 12345) { throw 'Unexpected process id.' }
    $global:Vpn233RuntimeTestState.Stopped++
}
function Get-CimInstance {
    param([string]$ClassName, [string]$Filter, [string]$ErrorAction)
    if ($ClassName -ne 'Win32_Process' -or $Filter -ne 'ProcessId=12345') { throw 'Unexpected process query.' }
    return [pscustomobject]@{ CommandLine = $global:Vpn233RuntimeTestState.Command }
}
function Start-Process {
    param([string]$FilePath, [object[]]$ArgumentList, [string]$WorkingDirectory, [string]$WindowStyle, [switch]$PassThru, [string]$RedirectStandardOutput, [string]$RedirectStandardError)
    $global:Vpn233RuntimeTestState.Started += [pscustomobject]@{ Binary = $FilePath; Arguments = ($ArgumentList -join ' ') }
    $fake = [pscustomobject]@{ Id = (4000 + $global:Vpn233RuntimeTestState.Started.Count); StartTime = [datetime]::UtcNow; HasExited = $false }
    $fake | Add-Member -MemberType ScriptMethod -Name Refresh -Value { return }
    return $fake
}
try {
    New-Item -ItemType Directory -Force -Path (Join-Path $work 'runtime'), (Join-Path $work 'bin') | Out-Null
    $env:VPN233_BIN_DIR = Join-Path $work 'bin'
    $env:VPN233_CONFIG_DIR = Join-Path $work 'config'
    $global:Vpn233RuntimeTestState.Process = [pscustomobject]@{ Path = (Join-Path $work 'bin/vpn233-provider-server.exe'); StartTime = [datetime]::UtcNow; HasExited = $true }
    $global:Vpn233RuntimeTestState.Process | Add-Member -MemberType ScriptMethod -Name WaitForExit -Value { param($timeout) return $timeout -gt 0 }
    $global:Vpn233RuntimeTestState.Process | Add-Member -MemberType ScriptMethod -Name Refresh -Value { return }
    $pidFile = Join-Path $work 'runtime/provider-server.pid'
    [IO.File]::WriteAllText($pidFile, '12345')
    [IO.File]::WriteAllText(($pidFile + '.identity'), 'not-the-same-process-start')
    & (Join-Path $repository 'stop.ps1') -InstallDir $work
    Assert-True 'reused PID with different start identity never stopped' ($global:Vpn233RuntimeTestState.Stopped -eq 0)
    [IO.File]::WriteAllText($pidFile, '12345')
    $global:Vpn233RuntimeTestState.Command = 'vpn233-provider-server.exe --config C:\another-install\config.yaml'
    & (Join-Path $repository 'stop.ps1') -InstallDir $work
    Assert-True 'legacy PID with another config never stopped' ($global:Vpn233RuntimeTestState.Stopped -eq 0)
    [IO.File]::WriteAllText($pidFile, '12345')
    [IO.File]::WriteAllText(($pidFile + '.identity'), [string]$global:Vpn233RuntimeTestState.Process.StartTime.ToUniversalTime().Ticks)
    & (Join-Path $repository 'stop.ps1') -InstallDir $work
    Assert-True 'matching executable and process start identity stopped' ($global:Vpn233RuntimeTestState.Stopped -eq 1)
    Assert-True 'completed stop removes PID and identity records' (-not (Test-Path -LiteralPath $pidFile) -and -not (Test-Path -LiteralPath ($pidFile + '.identity')))
    . (Join-Path $repository 'scripts/lib/layout.ps1')
    $customBin = Join-Path $work 'external binaries'
    $customConfig = Join-Path $work 'external configuration'
    New-Item -ItemType Directory -Path $customBin, $customConfig | Out-Null
    foreach ($name in @('provider-server', 'subscribe-server', 'vpn233-client')) {
        $binary = if ($name -eq 'vpn233-client') { $name } else { 'vpn233-' + $name }
        [IO.File]::WriteAllText((Join-Path $customBin ($binary + '.exe')), 'synthetic payload')
        [IO.File]::WriteAllText((Join-Path $customConfig ($name + '.yaml')), 'synthetic configuration')
    }
    Save-Vpn233Layout $work user $customBin (Join-Path $work 'web') $customConfig (Join-Path $work 'data')
    $env:VPN233_BIN_DIR = $null; $env:VPN233_CONFIG_DIR = $null
    $global:Vpn233RuntimeTestState.Process.Path = Join-Path $customBin 'vpn233-provider-server.exe'
    [IO.File]::WriteAllText($pidFile, '12345')
    [IO.File]::WriteAllText(($pidFile + '.identity'), [string]$global:Vpn233RuntimeTestState.Process.StartTime.ToUniversalTime().Ticks)
    & (Join-Path $repository 'stop.ps1') -InstallDir $work
    Assert-True 'runtime stop reads custom paths from saved layout without environment' ($global:Vpn233RuntimeTestState.Stopped -eq 2)
    & (Join-Path $repository 'start.ps1') -InstallDir $work
    Assert-True 'runtime start launches three binaries from saved custom directory' ($global:Vpn233RuntimeTestState.Started.Count -eq 3 -and @($global:Vpn233RuntimeTestState.Started | Where-Object { -not $_.Binary.StartsWith($customBin) }).Count -eq 0)
    Assert-True 'runtime start uses saved external config paths' (@($global:Vpn233RuntimeTestState.Started | Where-Object { -not $_.Arguments.Contains($customConfig) }).Count -eq 0)
    Write-Host ('test-services.ps1: ' + $script:passed + ' passed')
} finally {
    $env:VPN233_BIN_DIR = $previousBin
    $env:VPN233_CONFIG_DIR = $previousConfig
    Remove-Variable -Name Vpn233RuntimeTestState -Scope Global
    Remove-Item -LiteralPath $work -Recurse -Force
}
