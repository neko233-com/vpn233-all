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
    if ($env:OS -ne 'Windows_NT') {
        foreach ($directory in @('runtime', 'logs')) {
            Assert-True ('private ' + $directory + ' directory is mode 700') (([int][IO.File]::GetUnixFileMode((Join-Path $work $directory))) -eq 448)
        }
        $stateFiles = @(Get-ChildItem -LiteralPath (Join-Path $work 'runtime') -File)
        Assert-True 'all PID and identity files are mode 600' ($stateFiles.Count -eq 6 -and @($stateFiles | Where-Object { ([int][IO.File]::GetUnixFileMode($_.FullName)) -ne 384 }).Count -eq 0)
    }
    if ($env:OS -eq 'Windows_NT') {
        # Exercise the installed scripts in a native Windows PowerShell 5.1
        # process. Advanced-script parameter defaults see an empty PSScriptRoot
        # there, unlike PowerShell 7; invoking the scripts from a PS7 test alone
        # cannot cover that behavior. Only business-process operations are mocked.
        $nativeFixture = Join-Path $work 'native PS5 installed directory'
        $nativeOther = Join-Path $work 'native PS5 explicit directory'
        foreach ($fixture in @($nativeFixture, $nativeOther)) {
            foreach ($directory in @('bin', 'config', 'data', 'scripts/lib')) { New-Item -ItemType Directory -Force -Path (Join-Path $fixture $directory) | Out-Null }
            foreach ($name in @('provider-server', 'subscribe-server', 'vpn233-client')) {
                $program = if ($name -eq 'vpn233-client') { $name } else { 'vpn233-' + $name }
                [IO.File]::WriteAllText((Join-Path $fixture ('bin/' + $program + '.exe')), 'synthetic process fixture')
                [IO.File]::WriteAllText((Join-Path $fixture ('config/' + $name + '.yaml')), 'synthetic configuration')
            }
            [IO.File]::WriteAllText((Join-Path $fixture 'version.txt'), 'synthetic fixture version')
            [IO.File]::WriteAllText((Join-Path $fixture 'data/preserved.txt'), 'synthetic preserved data')
        }
        Copy-Item -LiteralPath (Join-Path $repository 'scripts/lib/layout.ps1') -Destination (Join-Path $nativeFixture 'scripts/lib/layout.ps1')
        # Load fixture functions only after the production param block has been
        # bound. A wrapper invoking & start.ps1 would hide the PS5 -File defect.
        foreach ($name in @('start.ps1', 'stop.ps1', 'uninstall.ps1')) {
            $nativeScript = [IO.File]::ReadAllText((Join-Path $repository $name))
            $nativeScript = $nativeScript.Replace('Set-StrictMode -Version Latest', ('. (Join-Path $PSScriptRoot ''native-process-fixture.ps1'')' + [Environment]::NewLine + 'Set-StrictMode -Version Latest'))
            $nativeScript += [Environment]::NewLine + 'Save-NativeServicesFixture' + [Environment]::NewLine
            [IO.File]::WriteAllText((Join-Path $nativeFixture $name), $nativeScript, (New-Object Text.UTF8Encoding($false)))
        }
        $nativeReport = Join-Path $nativeFixture 'native-state.json'
        $nativeSource = @'
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -ne 5) { throw 'Expected native Windows PowerShell 5.' }
$global:NativeServicesReport = Join-Path $PSScriptRoot 'native-state.json'
$global:NativeServicesFixture = if (Test-Path -LiteralPath $global:NativeServicesReport) { [IO.File]::ReadAllText($global:NativeServicesReport) | ConvertFrom-Json } else { [pscustomobject]@{ Started = @(); Stopped = 0; Records = @() } }
function Save-NativeServicesFixture {
    [IO.File]::WriteAllText($global:NativeServicesReport, ($global:NativeServicesFixture | ConvertTo-Json -Depth 5 -Compress))
}
function New-NativeProcessFixture {
    param($Record)
    $fake = [pscustomobject]@{ Id = $Record.Id; Path = $Record.Path; StartTime = [datetime]::new([long]$Record.StartTicks, [DateTimeKind]::Utc); HasExited = $Record.HasExited }
    $fake | Add-Member -MemberType ScriptMethod -Name Refresh -Value { $processId = $this.Id; $this.HasExited = @($global:NativeServicesFixture.Records | Where-Object { $_.Id -eq $processId })[0].HasExited }
    $fake | Add-Member -MemberType ScriptMethod -Name WaitForExit -Value { param($timeout) return $timeout -gt 0 }
    return $fake
}
function Start-Process {
    param([string]$FilePath, [object[]]$ArgumentList, [string]$WorkingDirectory, [string]$WindowStyle, [switch]$PassThru, [string]$RedirectStandardOutput, [string]$RedirectStandardError)
    $state = $global:NativeServicesFixture
    $state.Started += [pscustomobject]@{ Binary = $FilePath; Arguments = ($ArgumentList -join ' '); Directory = $WorkingDirectory }
    $record = [pscustomobject]@{ Id = (5000 + $state.Started.Count); Path = $FilePath; StartTicks = [datetime]::UtcNow.Ticks; HasExited = $false }
    $state.Records += $record
    return New-NativeProcessFixture $record
}
function Get-Process {
    param([int]$Id, [string]$ErrorAction)
    $record = @($global:NativeServicesFixture.Records | Where-Object { $_.Id -eq $Id })
    if ($record.Count -eq 1) { return New-NativeProcessFixture $record[0] }
    return $null
}
function Stop-Process {
    param([int]$Id, [string]$ErrorAction)
    $record = @($global:NativeServicesFixture.Records | Where-Object { $_.Id -eq $Id })
    if ($record.Count -ne 1) { throw 'Unexpected synthetic process identity.' }
    $record[0].HasExited = $true
    $global:NativeServicesFixture.Stopped++
}
'@
        [IO.File]::WriteAllText((Join-Path $nativeFixture 'native-process-fixture.ps1'), $nativeSource, (New-Object Text.UTF8Encoding($false)))
        function Invoke-NativeServiceFixture {
            param([string]$ScriptName, [string]$InstallDirectory = '')
            $nativeInfo = New-Object Diagnostics.ProcessStartInfo
            $nativeInfo.FileName = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
            $nativeInfo.Arguments = '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + (Join-Path $nativeFixture $ScriptName) + '"'
            if ($InstallDirectory) { $nativeInfo.Arguments += ' -InstallDir "' + $InstallDirectory + '"' }
            $nativeInfo.WorkingDirectory = [IO.Path]::GetTempPath()
            $nativeInfo.UseShellExecute = $false
            $nativeInfo.CreateNoWindow = $true
            $nativeInfo.RedirectStandardOutput = $true
            $nativeInfo.RedirectStandardError = $true
            # A PS7-only module search path must not contaminate native PS5.
            $nativeAllow = @('PATH', 'HOME', 'USERPROFILE', 'LANG', 'LC_ALL', 'SYSTEMROOT', 'WINDIR', 'COMSPEC', 'TEMP', 'TMP', 'PROGRAMFILES', 'PROGRAMFILES(X86)', 'LOCALAPPDATA', 'APPDATA', 'PATHEXT', 'OS', 'PROCESSOR_ARCHITECTURE', 'PROCESSOR_ARCHITEW6432')
            foreach ($key in @($nativeInfo.EnvironmentVariables.Keys)) { if ($key.ToUpperInvariant() -notin $nativeAllow) { $nativeInfo.EnvironmentVariables.Remove($key) } }
            $nativeProcess = New-Object Diagnostics.Process
            try {
                $nativeProcess.StartInfo = $nativeInfo
                [void]$nativeProcess.Start()
                $nativeStdout = $nativeProcess.StandardOutput.ReadToEndAsync()
                $nativeStderr = $nativeProcess.StandardError.ReadToEndAsync()
                if (-not $nativeProcess.WaitForExit(15000)) { $nativeProcess.Kill(); $nativeProcess.WaitForExit(); throw 'Native PS5 service fixture exceeded its deadline.' }
                if ($nativeProcess.ExitCode -ne 0 -or -not $nativeStdout.Wait(5000) -or -not $nativeStderr.Wait(5000)) { throw 'Native PS5 installed script did not complete successfully.' }
            } finally { $nativeProcess.Dispose() }
        }
        Invoke-NativeServiceFixture 'start.ps1'
        $nativeResult = [IO.File]::ReadAllText($nativeReport) | ConvertFrom-Json
        Assert-True 'native PS5 no-argument start uses its installed directory for three processes' ($nativeResult.Started.Count -eq 3 -and @($nativeResult.Started | Where-Object { $_.Directory -ne $nativeFixture -or -not $_.Binary.StartsWith((Join-Path $nativeFixture 'bin') + [IO.Path]::DirectorySeparatorChar) -or -not $_.Arguments.Contains((Join-Path $nativeFixture 'config')) }).Count -eq 0)
        Invoke-NativeServiceFixture 'stop.ps1'
        $nativeResult = [IO.File]::ReadAllText($nativeReport) | ConvertFrom-Json
        Assert-True 'native PS5 no-argument stop owns and stops all three identities' ($nativeResult.Stopped -eq 3 -and @(Get-ChildItem -LiteralPath (Join-Path $nativeFixture 'runtime') -File).Count -eq 0)
        Invoke-NativeServiceFixture 'uninstall.ps1'
        Assert-True 'native PS5 no-argument uninstall preserves configuration and data' (@(Get-ChildItem -LiteralPath (Join-Path $nativeFixture 'bin') -File).Count -eq 0 -and @(Get-ChildItem -LiteralPath (Join-Path $nativeFixture 'config') -File).Count -eq 3 -and [IO.File]::ReadAllText((Join-Path $nativeFixture 'data/preserved.txt')) -eq 'synthetic preserved data')
        Invoke-NativeServiceFixture 'start.ps1' $nativeOther
        $nativeResult = [IO.File]::ReadAllText($nativeReport) | ConvertFrom-Json
        $explicitStarted = @($nativeResult.Started | Select-Object -Last 3)
        Assert-True 'native PS5 explicit install directory still starts three processes' ($nativeResult.Started.Count -eq 6 -and @($explicitStarted | Where-Object { $_.Directory -ne $nativeOther -or -not $_.Binary.StartsWith((Join-Path $nativeOther 'bin') + [IO.Path]::DirectorySeparatorChar) -or -not $_.Arguments.Contains((Join-Path $nativeOther 'config')) }).Count -eq 0)
        Invoke-NativeServiceFixture 'stop.ps1' $nativeOther
        $nativeResult = [IO.File]::ReadAllText($nativeReport) | ConvertFrom-Json
        Assert-True 'native PS5 explicit install directory still stops three identities' ($nativeResult.Stopped -eq 6 -and @(Get-ChildItem -LiteralPath (Join-Path $nativeOther 'runtime') -File).Count -eq 0)
    }
    Write-Host ('test-services.ps1: ' + $script:passed + ' passed')
} finally {
    $env:VPN233_BIN_DIR = $previousBin
    $env:VPN233_CONFIG_DIR = $previousConfig
    Remove-Variable -Name Vpn233RuntimeTestState -Scope Global
    Remove-Item -LiteralPath $work -Recurse -Force
}
