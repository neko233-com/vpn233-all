[CmdletBinding()]
param([string]$InstallDir = $PSScriptRoot)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$base = [IO.Path]::GetFullPath($InstallDir)
$layout = $null
$layoutHelper = Join-Path $PSScriptRoot 'scripts/lib/layout.ps1'
if (Test-Path -LiteralPath $layoutHelper) { . $layoutHelper; $layout = Read-Vpn233Layout $base }
elseif (Test-Path -LiteralPath (Join-Path $base '.vpn233-layout')) { throw 'Missing public layout parser.' }
$binaryDir = if ($env:VPN233_BIN_DIR) { $env:VPN233_BIN_DIR } elseif ($layout) { $layout['bin'] } elseif (Test-Path (Join-Path $base 'bin')) { Join-Path $base 'bin' } elseif (Test-Path (Join-Path $base 'generated')) { Join-Path $base 'generated' } else { Join-Path $base 'target/release' }
$configDir = if ($env:VPN233_CONFIG_DIR) { $env:VPN233_CONFIG_DIR } elseif ($layout) { $layout['config'] } else { Join-Path $base 'config' }
$runDir = Join-Path $base 'runtime'
$logDir = Join-Path $base 'logs'
$windowsRuntime = $env:OS -eq 'Windows_NT'
$null = New-Item -ItemType Directory -Force -Path $runDir, $logDir
foreach ($privateDirectory in @($runDir, $logDir)) {
    if ($windowsRuntime) {
        $acl = Get-Acl -LiteralPath $privateDirectory
        $allowed = @([Security.Principal.WindowsIdentity]::GetCurrent().User.Value, 'S-1-5-18', 'S-1-5-32-544')
        $unexpected = @($acl.Access | Where-Object { $_.AccessControlType -eq 'Allow' -and $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value -notin $allowed })
        if (-not $acl.AreAccessRulesProtected -or $unexpected.Count) {
            $acl.SetAccessRuleProtection($true, $false)
            foreach ($rule in @($acl.Access)) { [void]$acl.RemoveAccessRuleSpecific($rule) }
            foreach ($sid in $allowed) { $acl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule((New-Object Security.Principal.SecurityIdentifier $sid), 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow'))) }
            Set-Acl -LiteralPath $privateDirectory -AclObject $acl
        }
    } else {
        & chmod 700 -- $privateDirectory
        if ($LASTEXITCODE -ne 0) { throw 'Unable to protect private process directories.' }
    }
}
$services = @{'provider-server' = 'vpn233-provider-server'; 'subscribe-server' = 'vpn233-subscribe-server'; 'vpn233-client' = 'vpn233-client'}
foreach ($name in $services.Keys) {
    $binary = Join-Path $binaryDir ($services[$name] + '.exe')
    $config = Join-Path $configDir ($name + '.yaml')
    if ($config -match '["\r\n]') { throw 'Configuration paths cannot contain quotes or line breaks.' }
    if (-not (Test-Path -LiteralPath $binary) -or -not (Test-Path -LiteralPath $config)) { throw "Missing binary or configuration for $name; run install.ps1 first." }
}
foreach ($name in $services.Keys) {
    $binary = [IO.Path]::GetFullPath((Join-Path $binaryDir ($services[$name] + '.exe')))
    $config = [IO.Path]::GetFullPath((Join-Path $configDir ($name + '.yaml')))
    $pidFile = Join-Path $runDir ($name + '.pid')
    if (Test-Path -LiteralPath $pidFile) {
        $processId = 0
        if ([int]::TryParse((Get-Content -LiteralPath $pidFile -Raw).Trim(), [ref]$processId)) {
            $running = Get-Process -Id $processId -ErrorAction SilentlyContinue
            if ($running -and $running.Path -eq $binary) {
                $identityFile = $pidFile + '.identity'
                if (Test-Path -LiteralPath $identityFile -PathType Leaf) {
                    if ((Get-Content -LiteralPath $identityFile -Raw).Trim() -eq [string]$running.StartTime.ToUniversalTime().Ticks) { continue }
                } else {
                    $command = Get-CimInstance -ClassName Win32_Process -Filter ('ProcessId=' + $processId) -ErrorAction Stop
                    $configPattern = '(?:^|\s)--config\s+(?:"' + [regex]::Escape($config) + '"|' + [regex]::Escape($config) + ')(?=\s|$)'
                    if ($command -and $command.CommandLine -match $configPattern) { continue }
                }
            }
        }
    }
    $process = Start-Process -FilePath $binary -ArgumentList @('--config', ('"' + $config + '"')) -WorkingDirectory $base -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $logDir ($name + '.out.log')) -RedirectStandardError (Join-Path $logDir ($name + '.err.log'))
    Set-Content -LiteralPath $pidFile -Value $process.Id -Encoding ASCII
    Set-Content -LiteralPath ($pidFile + '.identity') -Value $process.StartTime.ToUniversalTime().Ticks -Encoding ASCII
    if (-not $windowsRuntime) {
        foreach ($stateFile in @($pidFile, ($pidFile + '.identity'))) {
            & chmod 600 -- $stateFile
            if ($LASTEXITCODE -ne 0) { throw 'Unable to protect private process identity records.' }
        }
    }
    Start-Sleep -Milliseconds 100
    $process.Refresh()
    if ($process.HasExited) { throw 'A VPN233 process exited during startup. Inspect private logs locally.' }
}
Write-Host 'VPN233 processes started. Logs stay in the local logs directory.'
