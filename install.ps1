<# Windows PowerShell 5.1 / PowerShell 7. Use -Help for supported parameters.
Examples: .\install.ps1 -Release -Mirror auto
          .\install.ps1 -Bundle bundle.zip -Sha256 HASH -NoStart
No arguments in a checkout installs the local build without starting services.
#>
[CmdletBinding()]
param(
    [ValidateSet('install', 'update')][string]$Action = 'install',
    [switch]$Release, [string]$Version = '', [string]$Bundle = '',
    [string]$Sha256 = '', [string]$Prefix = '', [switch]$System,
    [string]$Mirror = '', [switch]$NoStart, [switch]$Yes, [switch]$DryRun, [switch]$Help
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($Help) {
    Write-Host 'Usage: install.ps1 [-Release] [-Version 2.1.0] [-Bundle FILE] [-Sha256 HASH]'
    Write-Host '                   [-Prefix DIR] [-System] [-Mirror auto|off|PREFIX] [-NoStart] [-Yes] [-DryRun]'
    Write-Host 'No arguments installs the source build. Releases require SHA256SUMS; existing configuration/data are preserved.'
    return
}
$rootDir = if ($PSScriptRoot) { $PSScriptRoot } else { [IO.Path]::GetTempPath() }
$binaryNames = @('vpn233-provider-server', 'vpn233-subscribe-server', 'vpn233-client')
$configNames = @('provider-server', 'subscribe-server', 'vpn233-client')
$releaseMode = $Release -or $Version -or $Bundle -or $Prefix -or $System
$temporaryDir = ''
$generatedCredentials = $false
$rollbackFiles = @()
$rollbackActive = $false
$webPrevious = ''
$webInstalled = $false
$webStaged = ''
$layoutLibrary = ''
$library = @((Join-Path $rootDir 'scripts/lib/mirrors.ps1'), (Join-Path $rootDir 'lib/mirrors.ps1')) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1

function Get-Vpn233Setting {
    param([string]$Name, [string]$Fallback)
    $value = [Environment]::GetEnvironmentVariable($Name)
    if ($value) { return $value }
    return $Fallback
}
function New-Vpn233Secret {
    $bytes = New-Object byte[] 24
    $generator = [Security.Cryptography.RandomNumberGenerator]::Create()
    try { $generator.GetBytes($bytes) } finally { $generator.Dispose() }
    return ([BitConverter]::ToString($bytes)).Replace('-', '').ToLowerInvariant()
}
function Protect-Vpn233Directory {
    param([string]$Path)
    New-Item -ItemType Directory -Force -Path $Path | Out-Null
    if ($env:OS -eq 'Windows_NT') {
        $identity = [Security.Principal.WindowsIdentity]::GetCurrent().User
        $acl = Get-Acl -LiteralPath $Path
        $allowedSids = @($identity.Value, 'S-1-5-18', 'S-1-5-32-544')
        $unexpectedRules = @($acl.Access | Where-Object {
            $_.AccessControlType -eq 'Allow' -and $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value -notin $allowedSids
        })
        if ($acl.AreAccessRulesProtected -and $unexpectedRules.Count -eq 0) { return }
        $acl.SetAccessRuleProtection($true, $false)
        foreach ($existingRule in @($acl.Access)) { [void]$acl.RemoveAccessRuleSpecific($existingRule) }
        foreach ($sid in @($identity, (New-Object Security.Principal.SecurityIdentifier 'S-1-5-18'), (New-Object Security.Principal.SecurityIdentifier 'S-1-5-32-544'))) {
            $rule = New-Object Security.AccessControl.FileSystemAccessRule($sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')
            $acl.AddAccessRule($rule)
        }
        Set-Acl -LiteralPath $Path -AclObject $acl
    } else {
        & chmod 700 -- $Path
        if ($LASTEXITCODE -ne 0) { throw 'Failed to protect private directory permissions.' }
    }
}
function Write-Vpn233Text {
    param([string]$Path, [string]$Text)
    [IO.File]::WriteAllText($Path, $Text, (New-Object Text.UTF8Encoding $false))
}
function ConvertTo-Vpn233YamlString {
    param([AllowEmptyString()][string]$Value)
    if ($Value -match '[\x00-\x1f\x7f]') { throw 'Configuration values cannot contain control characters.' }
    return "'" + $Value.Replace("'", "''") + "'"
}
function Set-Vpn233YamlValue {
    param([string]$Text, [string]$KeyPattern, [string]$Value)
    $regex = New-Object Text.RegularExpressions.Regex ('(?m)^(' + $KeyPattern + '):[^\r\n]*')
    $literalValue = $Value
    $replacement = { param($match) return $match.Groups[1].Value + ': ' + $literalValue }.GetNewClosure()
    return $regex.Replace($Text, [Text.RegularExpressions.MatchEvaluator]$replacement)
}
function Get-Vpn233FreePort {
    param([int]$Start, [int[]]$Used)
    for ($port = $Start; $port -le [Math]::Min(65535, $Start + 199); $port++) {
        if ($Used -contains $port) { continue }
        $listener = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, $port)
        try { $listener.Start(); return $port } catch { Write-Verbose 'Port busy; trying next local port.' } finally { $listener.Stop() }
    }
    throw 'No free local TCP port found.'
}
function Get-Vpn233Port {
    param([string]$Config, [string]$Pattern, [string]$Environment, [int]$Default, [int[]]$Used)
    $portText = ''
    if (Test-Path -LiteralPath $Config -PathType Leaf) {
        $text = [IO.File]::ReadAllText($Config)
        if ($text -match $Pattern) { $portText = $Matches[1] }
    }
    if (-not $portText) { $portText = [Environment]::GetEnvironmentVariable($Environment) }
    if (-not $portText) { return Get-Vpn233FreePort $Default $Used }
    $port = 0
    if (-not [int]::TryParse($portText, [ref]$port) -or $port -lt 1 -or $port -gt 65535) { throw ('Invalid TCP port: ' + $Environment) }
    if ($Used -contains $port) { throw 'Configuration contains conflicting TCP ports.' }
    return $port
}
function Get-Vpn233BundleRoot {
    param([string]$Directory)
    foreach ($candidate in @($Directory) + @(Get-ChildItem -LiteralPath $Directory -Directory | ForEach-Object { $_.FullName })) {
        if ((Test-Path -LiteralPath (Join-Path $candidate 'version.txt') -PathType Leaf) -and (Test-Path -LiteralPath (Join-Path $candidate 'bin') -PathType Container)) { return $candidate }
    }
    throw 'Bundle must contain version.txt, bin/, examples/ and admin-web/dist/.'
}
function Expand-Vpn233Bundle {
    param([string]$Archive, [string]$Destination)
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    $destinationRoot = [IO.Path]::GetFullPath($Destination).TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    if ($Archive -match '(?i)\.zip$') {
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $zip = [IO.Compression.ZipFile]::OpenRead($Archive)
        try {
            foreach ($entry in $zip.Entries) {
                $name = $entry.FullName.Replace('\', '/')
                if ($name -match '(^/|(^|/)\.\.(/|$)|:)' -or (($entry.ExternalAttributes -shr 16) -band 0xf000) -eq 0xa000) { throw 'Archive contains an unsafe path or symbolic link.' }
                $target = [IO.Path]::GetFullPath((Join-Path $Destination $name))
                if (-not $target.StartsWith($destinationRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Archive path escapes the extraction directory.' }
            }
        } finally { $zip.Dispose() }
        [IO.Compression.ZipFile]::ExtractToDirectory($Archive, $Destination)
    } elseif ($Archive -match '(?i)\.(tar\.gz|tgz)$') {
        if (-not (Get-Command tar -ErrorAction SilentlyContinue)) { throw 'tar is required for .tar.gz bundles. Prefer the Windows .zip release.' }
        $entries = @(& tar -tzf $Archive)
        if ($LASTEXITCODE -ne 0) { throw 'Unable to inspect bundle archive.' }
        if (@($entries | Where-Object { $_ -match '(^[/\\]|(^|[/\\])\.\.([/\\]|$)|:)' }).Count) { throw 'Archive contains an unsafe path.' }
        $details = @(& tar -tvzf $Archive)
        if ($LASTEXITCODE -ne 0 -or @($details | Where-Object { $_ -notmatch '^[d-]' }).Count) { throw 'Archive contains a link or unsupported entry.' }
        & tar -xzf $Archive -C $Destination
        if ($LASTEXITCODE -ne 0) { throw 'Unable to extract bundle archive.' }
    } else { throw 'Expected a .zip or .tar.gz bundle.' }
}
function Confirm-Vpn233LocalBundle {
    param([string]$Path)
    $expected = if ($Sha256) { $Sha256 } else { $env:VPN233_BUNDLE_SHA256 }
    if (-not $expected) {
        foreach ($sidecar in @(($Path + '.sha256'), ($Path + '.sha256sum'))) {
            if (Test-Path -LiteralPath $sidecar -PathType Leaf) { $expected = ((Get-Content -LiteralPath $sidecar -TotalCount 1) -split '\s+')[0]; break }
        }
    }
    $sums = Join-Path (Split-Path -Parent $Path) 'SHA256SUMS'
    if (-not $expected -and (Test-Path -LiteralPath $sums -PathType Leaf)) { $expected = Get-Vpn233Checksum $sums ([IO.Path]::GetFileName($Path)) }
    if ($expected) { Test-Vpn233Checksum $Path $expected }
    elseif ($env:VPN233_ALLOW_UNSIGNED -eq '1') { Write-Warning 'Installing local archive without a checksum by explicit VPN233_ALLOW_UNSIGNED=1 opt-in.' }
    else { throw 'Missing SHA-256. Provide -Sha256 HASH, an archive.sha256 sidecar, or SHA256SUMS.' }
}
function Install-Vpn233File {
    param([string]$Source, [string]$Destination)
    if ([IO.Path]::GetFullPath($Source) -eq [IO.Path]::GetFullPath($Destination)) { return }
    $staged = $Destination + '.' + [guid]::NewGuid().ToString('N') + '.tmp'
    try { Copy-Item -LiteralPath $Source -Destination $staged; Move-Item -LiteralPath $staged -Destination $Destination -Force }
    finally { if (Test-Path -LiteralPath $staged) { Remove-Item -LiteralPath $staged -Force } }
}
function Resolve-Vpn233SavedLayout {
    param([string]$Base, [string]$AdditionalRoot = '')
    $script:layoutLibrary = @((Join-Path $rootDir 'scripts/lib/layout.ps1'), (Join-Path $Base 'scripts/lib/layout.ps1'), $(if ($AdditionalRoot) { Join-Path $AdditionalRoot 'scripts/lib/layout.ps1' })) | Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Leaf) } | Select-Object -First 1
    if ($script:layoutLibrary) { . $script:layoutLibrary; return Read-Vpn233Layout $Base }
    if (Test-Path -LiteralPath (Join-Path $Base '.vpn233-layout')) { throw 'Runtime layout exists but its public parser is missing.' }
    return $null
}

try {
    if (-not $library -and -not $DryRun) {
        $temporaryDir = Join-Path ([IO.Path]::GetTempPath()) ('vpn233-install-' + [guid]::NewGuid().ToString('N'))
        Protect-Vpn233Directory $temporaryDir
        $library = Join-Path $temporaryDir 'mirrors.ps1'
        $prefixes = @('https://ghproxy.net/', 'https://gh.llkk.cc/', 'https://ghfast.top/', 'https://gh-proxy.com/', '')
        if ($env:VPN233_NO_MIRROR -eq '1' -or $Mirror -eq 'off') { $prefixes = @('') }
        elseif ($Mirror -and $Mirror -ne 'auto') {
            $uri = $null
            if (-not [uri]::TryCreate($Mirror, [UriKind]::Absolute, [ref]$uri) -or $uri.Scheme -notin @('http', 'https') -or $uri.UserInfo -or $uri.Query -or $uri.Fragment) { throw 'Invalid mirror prefix.' }
            $prefixes = @($Mirror.TrimEnd('/') + '/')
        }
        foreach ($prefixValue in $prefixes) {
            try {
                Invoke-WebRequest -UseBasicParsing -Uri ($prefixValue + 'https://raw.githubusercontent.com/neko233-com/vpn233-all/main/scripts/lib/mirrors.ps1') -OutFile ($library + '.part') -TimeoutSec 30
                if ((Get-Item -LiteralPath ($library + '.part')).Length -gt 0) { Move-Item -LiteralPath ($library + '.part') -Destination $library -Force; break }
            } catch { Write-Verbose 'Public bootstrap source unavailable; trying next source.' }
        }
        if (-not (Test-Path -LiteralPath $library -PathType Leaf)) { throw 'Unable to download the public mirror helper. Use a complete release bundle.' }
    }
    if ($library) { . $library }
    if (-not $DryRun) {
        if ($Mirror) { Set-Vpn233MirrorMode $Mirror }
        if ($Version) { $Version = Get-Vpn233Version $Version }
        if ($Sha256 -and $Sha256 -notmatch '^[0-9a-fA-F]{64}$') { throw '-Sha256 requires exactly 64 hexadecimal characters.' }
        if ($Sha256 -and -not $Bundle) { throw '-Sha256 requires -Bundle.' }
    }
    if ($Action -eq 'update') {
        $updateArgs = @{} + $PSBoundParameters
        [void]$updateArgs.Remove('Action')
        & (Join-Path $rootDir 'update.ps1') @updateArgs
        return
    }
    $baseDir = if ($Prefix) { $Prefix } elseif ($releaseMode) { Get-Vpn233Setting 'VPN233_INSTALL_DIR' (Get-Vpn233Setting 'VPN233_HOME' (Join-Path $env:USERPROFILE '.vpn233')) } else { $rootDir }
    if ($System -and -not $Prefix) { $baseDir = Join-Path $env:ProgramData 'vpn233' }
    $baseDir = [IO.Path]::GetFullPath($baseDir)
    $savedLayout = Resolve-Vpn233SavedLayout $baseDir
    $scope = if ($System) { 'system' } elseif ($savedLayout) { $savedLayout['scope'] } elseif ($releaseMode) { 'user' } else { 'source' }
    $binDefault = Join-Path $baseDir $(if ($releaseMode) { 'bin' } else { 'generated' })
    if ($savedLayout) { $binDefault = $savedLayout['bin'] }
    elseif ($releaseMode -and -not (Test-Path -LiteralPath (Join-Path $baseDir 'bin')) -and (Test-Path -LiteralPath (Join-Path $baseDir 'generated'))) { $binDefault = Join-Path $baseDir 'generated' }
    $binDir = [IO.Path]::GetFullPath((Get-Vpn233Setting 'VPN233_BIN_DIR' $binDefault))
    $webDist = [IO.Path]::GetFullPath((Get-Vpn233Setting 'VPN233_WEB_DIR' $(if ($savedLayout) { $savedLayout['web'] } else { Join-Path $baseDir 'admin-web/dist' })))
    $configDir = [IO.Path]::GetFullPath((Get-Vpn233Setting 'VPN233_CONFIG_DIR' $(if ($savedLayout) { $savedLayout['config'] } else { Join-Path $baseDir 'config' })))
    $dataDir = [IO.Path]::GetFullPath((Get-Vpn233Setting 'VPN233_DATA_DIR' $(if ($savedLayout) { $savedLayout['data'] } else { Join-Path $baseDir 'data' })))
    if ($DryRun) {
        Write-Host ('DRY RUN: ' + $(if ($releaseMode) { 'verified release' } else { 'source build' }) + ' installation into ' + $baseDir)
        Write-Host 'Existing configuration/data preserved. No files, downloads, or services changed.'
        return
    }
    if ($scope -eq 'system' -and $env:OS -eq 'Windows_NT') {
        $principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
        if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw '-System requires an elevated PowerShell session.' }
    }
    if (-not $temporaryDir) {
        $temporaryDir = Join-Path ([IO.Path]::GetTempPath()) ('vpn233-install-' + [guid]::NewGuid().ToString('N'))
        Protect-Vpn233Directory $temporaryDir
    }
    if ($releaseMode) {
        if ($Bundle) {
            $Bundle = [IO.Path]::GetFullPath($Bundle)
            if (Test-Path -LiteralPath $Bundle -PathType Container) { throw 'Use a checksum-verified archive for -Bundle, not an unpacked directory.' }
            Confirm-Vpn233LocalBundle $Bundle
        } else {
            $asset = 'vpn233-rust-windows-x86_64.zip'
            if (-not [Environment]::Is64BitOperatingSystem -or $env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { throw 'Published Windows bundles require an x86_64 operating system.' }
            if (-not $Version) {
                $versionFile = Join-Path $temporaryDir 'version.txt'
                Receive-Vpn233ReleaseAsset 'version.txt' $versionFile '' -Quiet
                $Version = Get-Vpn233Version ([IO.File]::ReadAllText($versionFile).Trim())
            }
            $sumsFile = Join-Path $temporaryDir 'SHA256SUMS'
            Receive-Vpn233ReleaseAsset 'SHA256SUMS' $sumsFile $Version
            $expected = Get-Vpn233Checksum $sumsFile $asset
            $Bundle = Join-Path $temporaryDir $asset
            Receive-Vpn233ReleaseAsset $asset $Bundle $Version
            Test-Vpn233Checksum $Bundle $expected
        }
        $extract = Join-Path $temporaryDir 'extract'
        Expand-Vpn233Bundle $Bundle $extract
        $sourceRoot = Get-Vpn233BundleRoot $extract
        $installedVersion = Get-Vpn233Version ([IO.File]::ReadAllText((Join-Path $sourceRoot 'version.txt')).Trim())
        if ($Version -and $installedVersion -ne $Version) { throw 'Bundle version does not match the requested release.' }
        $binSource = Join-Path $sourceRoot 'bin'
        $webSource = Join-Path $sourceRoot 'admin-web/dist'
        $templates = if (Test-Path -LiteralPath (Join-Path $sourceRoot 'examples')) { Join-Path $sourceRoot 'examples' } else { Join-Path $sourceRoot 'config' }
        if (-not $layoutLibrary) { $null = Resolve-Vpn233SavedLayout $baseDir $sourceRoot }
    } else {
        $sourceRoot = $rootDir
        $installedVersion = Get-Vpn233Version ([IO.File]::ReadAllText((Join-Path $rootDir 'version.txt')).Trim())
        $binSource = $env:VPN233_SOURCE_BIN_DIR
        if (-not $binSource) {
            $binSource = @((Join-Path $rootDir 'bin'), (Join-Path $rootDir 'generated'), (Join-Path $rootDir 'target/release')) | Where-Object {
                $candidate = $_
                @($binaryNames | Where-Object { -not (Test-Path -LiteralPath (Join-Path $candidate ($_.ToString() + '.exe')) -PathType Leaf) }).Count -eq 0
            } | Select-Object -First 1
        }
        if (-not $binSource) {
            if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { throw 'Cargo is required to build missing local binaries.' }
            Push-Location $rootDir
            try { & cargo build --release --workspace; if ($LASTEXITCODE -ne 0) { throw 'Rust build failed.' } } finally { Pop-Location }
            $binSource = Join-Path $rootDir 'target/release'
        }
        $webSource = Join-Path $rootDir 'admin-web/dist'
        if (-not (Test-Path -LiteralPath (Join-Path $webSource 'index.html') -PathType Leaf)) {
            Push-Location (Join-Path $rootDir 'admin-web')
            try {
                & npm.cmd ci; if ($LASTEXITCODE -ne 0) { throw 'npm ci failed.' }
                & npm.cmd run build; if ($LASTEXITCODE -ne 0) { throw 'Admin web build failed.' }
            } finally { Pop-Location }
        }
        $templates = ''
    }
    foreach ($name in $binaryNames) { if (-not (Test-Path -LiteralPath (Join-Path $binSource ($name + '.exe')) -PathType Leaf)) { throw ('Missing binary: ' + $name) } }
    if (-not (Test-Path -LiteralPath (Join-Path $webSource 'index.html') -PathType Leaf)) { throw 'Missing compiled admin UI.' }
    $templatePaths = @{}
    foreach ($name in $configNames) {
        $template = if ($templates) { Join-Path $templates ($name + '.yaml') } else { Join-Path $rootDir ($name + '/config.example.yaml') }
        if (-not (Test-Path -LiteralPath $template -PathType Leaf)) { throw ('Missing public example: ' + $name) }
        $templatePaths[$name] = $template
    }
    $used = @()
    $providerPort = Get-Vpn233Port (Join-Path $configDir 'provider-server.yaml') '(?m)^listen_port:\s*(\d+)' 'VPN233_PROVIDER_PORT' 8080 $used; $used += $providerPort
    $subscribePort = Get-Vpn233Port (Join-Path $configDir 'subscribe-server.yaml') '(?m)^  http_port:\s*(\d+)' 'VPN233_SUBSCRIBE_PORT' 23444 $used; $used += $subscribePort
    $socksPort = Get-Vpn233Port (Join-Path $configDir 'vpn233-client.yaml') '(?m)^socks_port:\s*(\d+)' 'VPN233_CLIENT_SOCKS_PORT' 1080 $used; $used += $socksPort
    $httpPort = Get-Vpn233Port (Join-Path $configDir 'vpn233-client.yaml') '(?m)^http_port:\s*(\d+)' 'VPN233_CLIENT_HTTP_PORT' 1081 $used; $used += $httpPort
    $clientPort = Get-Vpn233Port (Join-Path $configDir 'vpn233-client.yaml') '(?m)^web_addr:[^\r\n]*:(\d+)' 'VPN233_CLIENT_WEB_PORT' 8082 $used
    $adminUser = Get-Vpn233Setting 'VPN233_ADMIN_USER' 'root'
    $adminSecret = Get-Vpn233Setting 'VPN233_ADMIN_PASSWORD' (New-Vpn233Secret)
    if ($adminSecret -in @('root', 'change-me', 'change-me-admin-password')) { throw 'Set a non-default VPN233_ADMIN_PASSWORD or omit it to generate a random password.' }
    $feedSecret = Get-Vpn233Setting 'VPN233_PROVIDER_FEED_TOKEN' (New-Vpn233Secret)
    $registrySecret = Get-Vpn233Setting 'VPN233_REGISTRY_TOKEN' (New-Vpn233Secret)
    foreach ($name in $configNames) {
        if (Test-Path -LiteralPath (Join-Path $configDir ($name + '.yaml'))) { continue }
        $text = [IO.File]::ReadAllText($templatePaths[$name])
        $indent = if ($name -eq 'subscribe-server') { '  ' } else { '' }
        $text = Set-Vpn233YamlValue $text ($indent + 'admin_user') (ConvertTo-Vpn233YamlString $adminUser)
        $text = Set-Vpn233YamlValue $text ($indent + 'admin_password') (ConvertTo-Vpn233YamlString $adminSecret)
        switch ($name) {
            'provider-server' {
                $text = Set-Vpn233YamlValue $text 'listen_addr' "'127.0.0.1'"
                $text = Set-Vpn233YamlValue $text 'listen_port' $providerPort
                $text = Set-Vpn233YamlValue $text 'provider_access_token' (ConvertTo-Vpn233YamlString $feedSecret)
                $text = Set-Vpn233YamlValue $text 'registry_token' (ConvertTo-Vpn233YamlString $registrySecret)
                $text = Set-Vpn233YamlValue $text 'registry_url' (ConvertTo-Vpn233YamlString "http://127.0.0.1:$subscribePort")
            }
            'subscribe-server' {
                $text = Set-Vpn233YamlValue $text '  host' "'127.0.0.1'"
                $text = Set-Vpn233YamlValue $text '  http_port' $subscribePort
                $text = Set-Vpn233YamlValue $text '  provider_registry_token' (ConvertTo-Vpn233YamlString $registrySecret)
                $text = Set-Vpn233YamlValue $text '  data_dir' (ConvertTo-Vpn233YamlString (Join-Path $dataDir 'subscribe-server'))
                $text = Set-Vpn233YamlValue $text '  web_dir' (ConvertTo-Vpn233YamlString $webDist)
                $text = $text.Replace('http://127.0.0.1:8080/api/v1/nodes', "http://127.0.0.1:$providerPort/api/v1/nodes")
                $text = Set-Vpn233YamlValue $text '    access_token' (ConvertTo-Vpn233YamlString $feedSecret)
            }
            'vpn233-client' {
                $text = Set-Vpn233YamlValue $text 'socks_port' $socksPort
                $text = Set-Vpn233YamlValue $text 'http_port' $httpPort
                $text = Set-Vpn233YamlValue $text 'web_addr' (ConvertTo-Vpn233YamlString "127.0.0.1:$clientPort")
                $text = Set-Vpn233YamlValue $text 'stats_file' (ConvertTo-Vpn233YamlString (Join-Path $dataDir 'vpn233-client/stats.json'))
            }
        }
        Write-Vpn233Text (Join-Path $temporaryDir ($name + '.yaml')) $text
    }
    foreach ($directory in @($baseDir, $binDir, (Split-Path -Parent $webDist))) { New-Item -ItemType Directory -Force -Path $directory | Out-Null }
    Protect-Vpn233Directory $configDir
    Protect-Vpn233Directory $dataDir
    Protect-Vpn233Directory (Join-Path $baseDir 'logs')
    if ($releaseMode) {
        # Keep only public application files in private rollback staging. Runtime
        # data and live configurations are never copied or replaced.
        $payloadFiles = @($binaryNames | ForEach-Object { Join-Path $binDir ($_ + '.exe') })
        $payloadFiles += @( 'version.txt', 'install.ps1', 'update.ps1', 'deploy.ps1', 'start.ps1', 'stop.ps1', 'uninstall.ps1', 'scripts/lib/mirrors.ps1', 'scripts/lib/layout.ps1' | ForEach-Object { Join-Path $baseDir $_ })
        foreach ($destinationFile in $payloadFiles) {
            $backupFile = Join-Path $temporaryDir ('rollback-' + [guid]::NewGuid().ToString('N'))
            $existed = Test-Path -LiteralPath $destinationFile -PathType Leaf
            if ($existed) { Copy-Item -LiteralPath $destinationFile -Destination $backupFile }
            $rollbackFiles += [pscustomobject]@{ Destination = [IO.Path]::GetFullPath($destinationFile); Backup = $backupFile; Existed = $existed }
        }
        $rollbackActive = $true
        $stopper = Join-Path $sourceRoot 'stop.ps1'
        if (-not (Test-Path -LiteralPath $stopper -PathType Leaf)) { $stopper = Join-Path $rootDir 'stop.ps1' }
        if (Test-Path -LiteralPath $stopper -PathType Leaf) {
            $previousBin = $env:VPN233_BIN_DIR
            try { $env:VPN233_BIN_DIR = $binDir; & $stopper -InstallDir $baseDir }
            finally { $env:VPN233_BIN_DIR = $previousBin }
        }
    }
    foreach ($name in $binaryNames) { Install-Vpn233File (Join-Path $binSource ($name + '.exe')) (Join-Path $binDir ($name + '.exe')) }
    if ([IO.Path]::GetFullPath($webSource) -ne [IO.Path]::GetFullPath($webDist)) {
        $webStaged = [IO.Path]::GetFullPath($webDist) + '.new-' + [guid]::NewGuid().ToString('N')
        New-Item -ItemType Directory -Path $webStaged | Out-Null
        Get-ChildItem -LiteralPath $webSource -Force | Copy-Item -Destination $webStaged -Recurse -Force
        if (Test-Path -LiteralPath $webDist) {
            $webPrevious = [IO.Path]::GetFullPath($webDist) + '.old-' + [guid]::NewGuid().ToString('N')
            Move-Item -LiteralPath $webDist -Destination $webPrevious
        }
        Move-Item -LiteralPath $webStaged -Destination $webDist
        $webInstalled = $true
    }
    foreach ($name in $configNames) {
        $destination = Join-Path $configDir ($name + '.yaml')
        if (-not (Test-Path -LiteralPath $destination)) { Copy-Item -LiteralPath (Join-Path $temporaryDir ($name + '.yaml')) -Destination $destination; $generatedCredentials = $true }
    }
    if ($releaseMode) {
        foreach ($name in @('install.ps1', 'update.ps1', 'deploy.ps1', 'start.ps1', 'stop.ps1', 'uninstall.ps1')) {
            $source = Join-Path $sourceRoot $name
            if (Test-Path -LiteralPath $source -PathType Leaf) { Install-Vpn233File $source (Join-Path $baseDir $name) }
        }
        New-Item -ItemType Directory -Force -Path (Join-Path $baseDir 'scripts/lib') | Out-Null
        Install-Vpn233File $library (Join-Path $baseDir 'scripts/lib/mirrors.ps1')
        if ($layoutLibrary) { Install-Vpn233File $layoutLibrary (Join-Path $baseDir 'scripts/lib/layout.ps1') }
        Write-Vpn233Text (Join-Path $baseDir 'version.txt') ($installedVersion + "`n")
        if (-not $NoStart) {
            $starter = Join-Path $baseDir 'start.ps1'
            if (-not (Test-Path -LiteralPath $starter)) { throw 'Bundle is missing start.ps1. Files installed; start manually or rerun with -NoStart.' }
            $previousBin = $env:VPN233_BIN_DIR; $previousConfig = $env:VPN233_CONFIG_DIR
            try { $env:VPN233_BIN_DIR = $binDir; $env:VPN233_CONFIG_DIR = $configDir; & $starter }
            finally { $env:VPN233_BIN_DIR = $previousBin; $env:VPN233_CONFIG_DIR = $previousConfig }
            if ($env:VPN233_SKIP_HEALTHCHECK -ne '1') {
                foreach ($url in @("http://127.0.0.1:$providerPort/api/v1/health", "http://127.0.0.1:$subscribePort/api/status", "http://127.0.0.1:$clientPort/api/v1/health")) {
                    $healthy = $false
                    for ($attempt = 0; $attempt -lt 20; $attempt++) {
                        try { $response = Invoke-WebRequest -UseBasicParsing -Uri $url -TimeoutSec 2; $healthy = $response.StatusCode -eq 200; if ($healthy) { break } } catch { Write-Verbose 'Waiting for local service health.' }
                        Start-Sleep -Seconds 1
                    }
                    if (-not $healthy) { throw 'Installed service failed its health check. Inspect private logs locally.' }
                }
            }
        }
    }
    if (-not $layoutLibrary) { throw 'Missing public layout helper in this release bundle.' }
    . $layoutLibrary
    Save-Vpn233Layout $baseDir $scope $binDir $webDist $configDir $dataDir
    Write-Host ('VPN233 ' + $installedVersion + ' installed. Admin: http://127.0.0.1:' + $subscribePort + '/')
    Write-Host ('Existing data/configuration preserved. Private configuration: ' + $configDir)
    if ($generatedCredentials) { Write-Host 'Random credentials stored only in private configuration files; inspect locally with your editor.' }
    $rollbackActive = $false
} catch {
    if ($rollbackActive) {
        # Best-effort rollback retains failure status and never prints private logs.
        $stopper = Join-Path $baseDir 'stop.ps1'
        if (Test-Path -LiteralPath $stopper -PathType Leaf) {
            $previousBin = $env:VPN233_BIN_DIR
            try { $env:VPN233_BIN_DIR = $binDir; & $stopper -InstallDir $baseDir }
            catch { Write-Warning 'Unable to stop failed services before rollback; close them locally.' }
            finally { $env:VPN233_BIN_DIR = $previousBin }
        }
        foreach ($file in $rollbackFiles) {
            if ($file.Existed) { Copy-Item -LiteralPath $file.Backup -Destination $file.Destination -Force -ErrorAction Continue }
            elseif (Test-Path -LiteralPath $file.Destination -PathType Leaf) { Remove-Item -LiteralPath $file.Destination -Force -ErrorAction Continue }
        }
        if ($webInstalled -and (Test-Path -LiteralPath $webDist)) { Remove-Item -LiteralPath $webDist -Recurse -Force -ErrorAction Continue }
        if ($webPrevious -and (Test-Path -LiteralPath $webPrevious)) { Move-Item -LiteralPath $webPrevious -Destination $webDist -ErrorAction Continue; $webPrevious = '' }
        Write-Warning 'Application files rolled back after installation failure. Existing configuration/data preserved; restart the previous version locally.'
    }
    throw
} finally {
    if ($webStaged -and (Test-Path -LiteralPath $webStaged)) { Remove-Item -LiteralPath $webStaged -Recurse -Force }
    if ($webPrevious -and (Test-Path -LiteralPath $webPrevious)) { Remove-Item -LiteralPath $webPrevious -Recurse -Force }
    if ($temporaryDir -and (Test-Path -LiteralPath $temporaryDir)) { Remove-Item -LiteralPath $temporaryDir -Recurse -Force }
}
