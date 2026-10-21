# Offline installer integration tests. Fixtures contain only synthetic data.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$work = Join-Path ([IO.Path]::GetTempPath()) ('vpn233-install-test-' + [guid]::NewGuid().ToString('N'))
$variables = @('VPN233_INSTALL_DIR', 'VPN233_HOME', 'VPN233_BIN_DIR', 'VPN233_WEB_DIR', 'VPN233_CONFIG_DIR', 'VPN233_DATA_DIR', 'VPN233_ADMIN_PASSWORD', 'VPN233_ADMIN_USER', 'VPN233_PROVIDER_FEED_TOKEN', 'VPN233_REGISTRY_TOKEN', 'VPN233_ALLOW_UNSIGNED', 'VPN233_BUNDLE_SHA256', 'VPN233_PROVIDER_PORT', 'VPN233_SUBSCRIBE_PORT', 'VPN233_CLIENT_SOCKS_PORT', 'VPN233_CLIENT_HTTP_PORT', 'VPN233_CLIENT_WEB_PORT', 'VPN233_UPDATE_BUNDLE', 'VPN233_UPDATE_SHA256', 'VPN233_UPDATE_URLS')
$saved = @{}
foreach ($variable in $variables) { $saved[$variable] = [Environment]::GetEnvironmentVariable($variable); [Environment]::SetEnvironmentVariable($variable, $null) }
New-Item -ItemType Directory -Path $work | Out-Null
$shell = (Get-Process -Id $PID).Path
$script:passed = 0
function Assert-True {
    param([string]$Name, [bool]$Condition)
    if (-not $Condition) { throw ('FAIL: ' + $Name) }
    $script:passed++; Write-Host ('PASS: ' + $Name)
}
function Invoke-FixtureInstall {
    param([string[]]$Arguments, [bool]$Success = $true, [string]$Script = 'install.ps1')
    # Windows PowerShell wraps native stderr as NativeCommandError. Capture it
    # without turning an expected child failure into a terminating harness error.
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = (& $shell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repository $Script) @Arguments 2>&1 | Out-String)
        $code = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previousPreference }
    if (($code -eq 0) -ne $Success) { throw ('Unexpected installer exit code ' + $code + ': ' + $output) }
    return $output
}
try {
    $fixture = Join-Path $work 'fixture'
    foreach ($directory in @('bin', 'examples', 'admin-web/dist')) { New-Item -ItemType Directory -Force -Path (Join-Path $fixture $directory) | Out-Null }
    foreach ($binary in @('vpn233-provider-server', 'vpn233-subscribe-server', 'vpn233-client')) { [IO.File]::WriteAllText((Join-Path $fixture ('bin/' + $binary + '.exe')), 'synthetic executable fixture') }
    foreach ($config in @('provider-server', 'subscribe-server', 'vpn233-client')) { Copy-Item -LiteralPath (Join-Path $repository ($config + '/config.example.yaml')) -Destination (Join-Path $fixture ('examples/' + $config + '.yaml')) }
    [IO.File]::WriteAllText((Join-Path $fixture 'admin-web/dist/index.html'), '<!doctype html><title>offline fixture</title>')
    [IO.File]::WriteAllText((Join-Path $fixture 'version.txt'), "2.1.0`n")
    $archive = Join-Path $work 'fixture.zip'
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [IO.Compression.ZipFile]::CreateFromDirectory($fixture, $archive)
    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash
    $prefix = Join-Path $work 'install with spaces'
    $common = @('-Bundle', $archive, '-Prefix', $prefix, '-NoStart')
    $null = Invoke-FixtureInstall ($common + @('-DryRun'))
    Assert-True 'dry run creates no install directory' (-not (Test-Path -LiteralPath $prefix))
    $null = Invoke-FixtureInstall $common $false
    Assert-True 'missing checksum refuses installation before writes' (-not (Test-Path -LiteralPath $prefix))
    $null = Invoke-FixtureInstall ($common + @('-Sha256', ('0' * 64))) $false
    Assert-True 'checksum mismatch refuses installation before writes' (-not (Test-Path -LiteralPath $prefix))
    $null = Invoke-FixtureInstall ($common + @('-Sha256', $hash, '-Version', '9.9.9')) $false
    Assert-True 'version mismatch refuses installation before writes' (-not (Test-Path -LiteralPath $prefix))
    $env:VPN233_PROVIDER_PORT = '70000'
    $null = Invoke-FixtureInstall ($common + @('-Sha256', $hash)) $false
    Assert-True 'invalid port refuses installation before writes' (-not (Test-Path -LiteralPath $prefix))
    $env:VPN233_PROVIDER_PORT = $null
    $env:VPN233_ADMIN_PASSWORD = 'root'
    $null = Invoke-FixtureInstall ($common + @('-Sha256', $hash)) $false
    Assert-True 'default admin password refused before writes' (-not (Test-Path -LiteralPath $prefix))
    $env:VPN233_ADMIN_PASSWORD = 'synthetic''quoted"password\with&dollar$'
    $output = Invoke-FixtureInstall ($common + @('-Sha256', $hash))
    Assert-True 'credentials never printed' (-not $output.Contains($env:VPN233_ADMIN_PASSWORD))
    Assert-True 'all binaries installed' (@(Get-ChildItem -LiteralPath (Join-Path $prefix 'bin') -Filter '*.exe').Count -eq 3)
    Assert-True 'compiled admin UI installed' (Test-Path -LiteralPath (Join-Path $prefix 'admin-web/dist/index.html'))
    $provider = [IO.File]::ReadAllText((Join-Path $prefix 'config/provider-server.yaml'))
    Assert-True 'YAML quotes preserve password characters' ($provider.Contains("admin_password: '" + $env:VPN233_ADMIN_PASSWORD.Replace("'", "''") + "'"))
    Assert-True 'provider uses local binding' ($provider.Contains("listen_addr: '127.0.0.1'"))
    $subscribe = [IO.File]::ReadAllText((Join-Path $prefix 'config/subscribe-server.yaml'))
    Assert-True 'subscribe uses local binding' ($subscribe.Contains("host: '127.0.0.1'"))
    Assert-True 'private data directory configured' ($subscribe.Contains((Join-Path $prefix 'data/subscribe-server')))
    $configPath = Join-Path $prefix 'config/provider-server.yaml'
    $before = (Get-FileHash -LiteralPath $configPath -Algorithm SHA256).Hash
    $dataPath = Join-Path $prefix 'data/retained.txt'
    [IO.File]::WriteAllText($dataPath, 'synthetic retained data')
    $env:VPN233_ADMIN_PASSWORD = 'replacement-must-not-overwrite'
    $null = Invoke-FixtureInstall ($common + @('-Sha256', $hash))
    Assert-True 'reinstall preserves configuration exactly' ((Get-FileHash -LiteralPath $configPath -Algorithm SHA256).Hash -eq $before)
    Assert-True 'reinstall preserves runtime data' ([IO.File]::ReadAllText($dataPath) -eq 'synthetic retained data')
    $null = Invoke-FixtureInstall ($common + @('-Sha256', ('0' * 64))) $false
    Assert-True 'bad update preserves prior configuration' ((Get-FileHash -LiteralPath $configPath -Algorithm SHA256).Hash -eq $before)
    $null = Invoke-FixtureInstall ($common + @('-Sha256', $hash)) $true 'update.ps1'
    Assert-True 'update shares verified installation path and preserves data' ([IO.File]::ReadAllText($dataPath) -eq 'synthetic retained data')
    $binaryPath = Join-Path $prefix 'bin/vpn233-provider-server.exe'
    $webPath = Join-Path $prefix 'admin-web/dist/index.html'
    [IO.File]::WriteAllText($binaryPath, 'previous payload must return after failure')
    [IO.File]::WriteAllText($webPath, 'previous web must return after failure')
    $null = Invoke-FixtureInstall @('-Bundle', $archive, '-Prefix', $prefix, '-Sha256', $hash) $false
    Assert-True 'activation failure rolls binaries back' ([IO.File]::ReadAllText($binaryPath) -eq 'previous payload must return after failure')
    Assert-True 'activation failure rolls admin UI back' ([IO.File]::ReadAllText($webPath) -eq 'previous web must return after failure')
    Assert-True 'activation failure preserves configuration and runtime data' ((Get-FileHash -LiteralPath $configPath -Algorithm SHA256).Hash -eq $before -and [IO.File]::ReadAllText($dataPath) -eq 'synthetic retained data')
    $evil = Join-Path $work 'traversal.zip'
    $zip = [IO.Compression.ZipFile]::Open($evil, [IO.Compression.ZipArchiveMode]::Create)
    try { $entry = $zip.CreateEntry('../escaped.txt'); $writer = New-Object IO.StreamWriter($entry.Open()); $writer.Write('must not escape'); $writer.Dispose() } finally { $zip.Dispose() }
    $evilHash = (Get-FileHash -LiteralPath $evil -Algorithm SHA256).Hash
    $null = Invoke-FixtureInstall @('-Bundle', $evil, '-Sha256', $evilHash, '-Prefix', (Join-Path $work 'evil-install'), '-NoStart') $false
    Assert-True 'path traversal rejected without escaped file' (-not (Test-Path -LiteralPath (Join-Path $work 'escaped.txt')))
    $null = Invoke-FixtureInstall @('-Bundle', $fixture, '-Prefix', (Join-Path $work 'directory-install'), '-NoStart') $false
    Assert-True 'unverified directory bundle rejected' (-not (Test-Path -LiteralPath (Join-Path $work 'directory-install')))
    Assert-True 'no configuration included in fixture bundle' (-not (Test-Path -LiteralPath (Join-Path $fixture 'config')))
    . (Join-Path $repository 'scripts/lib/layout.ps1')
    $customBin = Join-Path $work 'custom binaries'
    $customWeb = Join-Path $work 'custom admin web'
    $customConfig = Join-Path $work 'custom private configuration'
    $customData = Join-Path $work 'custom private data'
    Move-Item -LiteralPath (Join-Path $prefix 'bin') -Destination $customBin
    Move-Item -LiteralPath (Join-Path $prefix 'admin-web/dist') -Destination $customWeb
    Move-Item -LiteralPath (Join-Path $prefix 'config') -Destination $customConfig
    Move-Item -LiteralPath (Join-Path $prefix 'data') -Destination $customData
    Save-Vpn233Layout $prefix user $customBin $customWeb $customConfig $customData
    foreach ($scriptName in @('install.ps1', 'update.ps1', 'stop.ps1')) { Copy-Item -LiteralPath (Join-Path $repository $scriptName) -Destination (Join-Path $prefix $scriptName) -Force }
    $env:VPN233_UPDATE_BUNDLE = $archive; $env:VPN233_UPDATE_SHA256 = $hash
    $installedUpdater = Join-Path $prefix 'update.ps1'
    $output = (& $shell -NoProfile -ExecutionPolicy Bypass -File $installedUpdater -NoStart 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0) { throw ('Installed updater failed: ' + $output) }
    Assert-True 'no-argument installed updater preserves external config bytes' ((Get-FileHash -LiteralPath (Join-Path $customConfig 'provider-server.yaml')).Hash -eq $before)
    Assert-True 'no-argument installed updater preserves external runtime data' ([IO.File]::ReadAllText((Join-Path $customData 'retained.txt')) -eq 'synthetic retained data')
    Assert-True 'update never creates alternate default layout' (-not (Test-Path -LiteralPath (Join-Path $prefix 'config')) -and -not (Test-Path -LiteralPath (Join-Path $prefix 'bin')))
    $savedLayout = Read-Vpn233Layout $prefix
    Assert-True 'saved custom paths retained' ($savedLayout['config'] -eq $customConfig -and $savedLayout['bin'] -eq $customBin)
    Save-Vpn233Layout $prefix system $customBin $customWeb $customConfig $customData
    $output = (& $shell -NoProfile -ExecutionPolicy Bypass -File $installedUpdater -DryRun 2>&1 | Out-String)
    Assert-True 'system-scope installed updater dry run keeps target root' ($LASTEXITCODE -eq 0 -and $output.Contains($prefix))
    [IO.File]::WriteAllText((Join-Path $prefix '.vpn233-layout'), "vpn233-layout-v1`nscope`tuser`nbin`trelative/path`nweb`tC:\web`nconfig`tC:\config`ndata`tC:\data`n")
    $rejected = $false
    try { Read-Vpn233Layout $prefix | Out-Null } catch { $rejected = $true }
    Assert-True 'malformed saved layout rejected without evaluation' $rejected
    if ($env:OS -eq 'Windows_NT') {
        [IO.File]::WriteAllText((Join-Path $prefix '.vpn233-layout'), "vpn233-layout-v1`nscope`tuser`nbin`tC:relative`nweb`tC:\web`nconfig`tC:\config`ndata`tC:\data`n")
        $rejected = $false
        try { Read-Vpn233Layout $prefix | Out-Null } catch { $rejected = $true }
        Assert-True 'drive-relative Windows metadata path rejected' $rejected
    }
    Write-Host ('test-installers.ps1: ' + $script:passed + ' passed')
} finally {
    foreach ($variable in $variables) { [Environment]::SetEnvironmentVariable($variable, $saved[$variable]) }
    Remove-Item -LiteralPath $work -Recurse -Force
}
