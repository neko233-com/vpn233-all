# Windows end-to-end suite for the VPN233 Rust services.
#
# Starts real provider-server, subscribe-server and vpn233-client processes in a
# throw-away directory, exercises the admin API, every subscription format, the
# provider registry and both client proxy listeners, then checks the privacy
# guarantees (no secrets in logs, no recorded domains). It is the PowerShell
# counterpart of scripts/e2e/e2e.sh and is safe to run on a developer machine:
# it never writes to the repository and only binds loopback ports.
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/e2e/e2e.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/e2e/e2e.ps1 -BinDir C:\path\to\bin
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/e2e/e2e.ps1 -Keep
[CmdletBinding()]
param(
    [string]$BinDir = '',
    [string]$WebDir = '',
    [switch]$Keep,
    [int]$TimeoutSeconds = 180
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
foreach ($key in @('GH_TOKEN', 'GITHUB_TOKEN', 'GH_ENTERPRISE_TOKEN', 'GITHUB_ENTERPRISE_TOKEN')) {
    if (Test-Path -LiteralPath ('Env:' + $key)) { Remove-Item -LiteralPath ('Env:' + $key) }
}

# --- paths and state -----------------------------------------------------
$ScriptRoot = $PSScriptRoot
if ([string]::IsNullOrEmpty($ScriptRoot)) {
    $ScriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
}
$RootDir = Split-Path -Parent (Split-Path -Parent $ScriptRoot)
$XtaskBinary = $env:VPN233_XTASK_BIN
if ([string]::IsNullOrWhiteSpace($XtaskBinary)) { $XtaskBinary = Join-Path $RootDir 'target/debug/vpn233-xtask.exe' }
if (-not (Test-Path -LiteralPath $XtaskBinary -PathType Leaf)) { throw 'Prebuild vpn233-xtask or set VPN233_XTASK_BIN before E2E' }
if ([string]::IsNullOrEmpty($BinDir)) {
    $BinDir = Join-Path (Join-Path $RootDir 'target') 'release'
}

$WorkDir = Join-Path ([System.IO.Path]::GetTempPath()) ('vpn233-e2e-' + [guid]::NewGuid().ToString('N'))
$WorkDirUnix = $WorkDir.Replace('\', '/')
$ConfigDir = Join-Path $WorkDir 'config'
$LogDir = Join-Path $WorkDir 'logs'
$DataDir = Join-Path $WorkDir 'data'
$BuiltWebDir = $WebDir
if ([string]::IsNullOrWhiteSpace($BuiltWebDir)) { $BuiltWebDir = Join-Path $RootDir 'admin-web/dist' }
$WebDir = Join-Path $WorkDir 'web'
$null = New-Item -ItemType Directory -Path $WorkDir
$workspaceAcl = Get-Acl -LiteralPath $WorkDir
$workspaceAcl.SetAccessRuleProtection($true, $false)
foreach ($rule in @($workspaceAcl.Access)) { [void]$workspaceAcl.RemoveAccessRuleSpecific($rule) }
$workspaceOwner = [Security.Principal.WindowsIdentity]::GetCurrent().User
$workspaceAcl.SetOwner($workspaceOwner)
foreach ($sid in @($workspaceOwner, (New-Object Security.Principal.SecurityIdentifier 'S-1-5-18'), (New-Object Security.Principal.SecurityIdentifier 'S-1-5-32-544'))) {
    $workspaceAcl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')))
}
Set-Acl -LiteralPath $WorkDir -AclObject $workspaceAcl
$null = New-Item -ItemType Directory -Force -Path $ConfigDir, $LogDir, $DataDir, $WebDir
$null = New-Item -ItemType Directory -Force -Path (Join-Path $DataDir 'subscribe'), (Join-Path $DataDir 'client')
if (-not (Test-Path -LiteralPath (Join-Path $BuiltWebDir 'index.html'))) { throw 'Build admin-web before E2E; generated UI is required' }
Copy-Item -Path (Join-Path $BuiltWebDir '*') -Destination $WebDir -Recurse

$script:Processes = @()
$script:LogCopies = @()
$script:StepNames = @()
$script:StepMilliseconds = @()
$script:StepName = ''
$script:StepStart = $null

function Write-Note {
    param([string]$Message)
    Write-Host ("  - " + $Message)
}

function Step-Begin {
    param([string]$Name)
    $script:StepName = $Name
    $script:StepStart = Get-Date
    Write-Host ''
    Write-Host ("[" + $Name + "]")
}

function Step-End {
    $elapsed = [int](New-TimeSpan -Start $script:StepStart -End (Get-Date)).TotalMilliseconds
    $script:StepNames += $script:StepName
    $script:StepMilliseconds += $elapsed
    Write-Host ("  ok (" + $elapsed + " ms)")
}

function Get-RandomHex {
    param([int]$ByteCount)
    $generator = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $bytes = New-Object byte[] $ByteCount
        $generator.GetBytes($bytes)
        $builder = New-Object System.Text.StringBuilder
        foreach ($byte in $bytes) {
            $null = $builder.Append($byte.ToString('x2'))
        }
        return $builder.ToString()
    }
    finally {
        $generator.Dispose()
    }
}

function Get-FreePort {
    $attempt = 0
    while ($attempt -lt 20) {
        $attempt = $attempt + 1
        $listener = New-Object System.Net.Sockets.TcpListener([System.Net.IPAddress]::Loopback, 0)
        try {
            $listener.Start()
            return $listener.LocalEndpoint.Port
        }
        catch {
            Start-Sleep -Milliseconds 50
        }
        finally {
            $listener.Stop()
        }
    }
    throw 'could not allocate a free loopback port'
}

function Get-CurlNoProxyArgument {
    # Legacy native invocation drops an empty argument unless it is quoted for
    # the native command line. Modern argument passing preserves it directly.
    $mode = Get-Variable -Name PSNativeCommandArgumentPassing -ErrorAction SilentlyContinue
    if ($null -eq $mode -or [string]$mode.Value -eq 'Legacy') { return '""' }
    return ''
}

function Wait-HttpReady {
    param([string]$Url, [int]$Seconds)
    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        try {
            $null = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 2 -ErrorAction Stop
            return $true
        }
        catch {
            $response = $_.Exception.Response
            if ($null -ne $response) {
                return $true
            }
            Start-Sleep -Milliseconds 200
        }
    }
    return $false
}

function Invoke-Api {
    param(
        [string]$Method,
        [string]$Url,
        [hashtable]$Headers = @{},
        [string]$Body = ''
    )
    $result = [pscustomobject]@{
        Status  = 0
        Content = ''
        Headers = @{}
    }
    try {
        $arguments = @{
            Uri                = $Url
            Method             = $Method
            UseBasicParsing    = $true
            TimeoutSec         = 20
            ErrorAction        = 'Stop'
        }
        if ($Headers.Count -gt 0) {
            $arguments['Headers'] = $Headers
        }
        if (-not [string]::IsNullOrEmpty($Body)) {
            $arguments['Body'] = [System.Text.Encoding]::UTF8.GetBytes($Body)
            $arguments['ContentType'] = 'application/json'
        }
        $response = Invoke-WebRequest @arguments
        $result.Status = [int]$response.StatusCode
        $result.Content = [string]$response.Content
        foreach ($key in $response.Headers.Keys) {
            $result.Headers[([string]$key).ToLowerInvariant()] = [string]$response.Headers[$key]
        }
    }
    catch {
        $webResponse = $_.Exception.Response
        if ($null -eq $webResponse) {
            throw ('request failed for ' + $Method + ' (URL and response redacted)')
        }
        $result.Status = [int]$webResponse.StatusCode
        $stream = $webResponse.GetResponseStream()
        $reader = New-Object System.IO.StreamReader($stream)
        try {
            $result.Content = $reader.ReadToEnd()
        }
        finally {
            $reader.Close()
        }
        foreach ($key in $webResponse.Headers.Keys) {
            $result.Headers[([string]$key).ToLowerInvariant()] = [string]$webResponse.Headers[$key]
        }
    }
    return $result
}

function Assert-Status {
    param($Response, [int]$Expected, [string]$Label)
    if ($Response.Status -ne $Expected) {
        throw ($Label + ': expected HTTP ' + $Expected + ', got ' + $Response.Status + ' (response redacted)')
    }
}

function Assert-HeaderContains {
    param($Response, [string]$Name, [string]$Needle, [string]$Label)
    $key = $Name.ToLowerInvariant()
    if (-not $Response.Headers.ContainsKey($key)) {
        throw ($Label + ': missing response header ' + $Name)
    }
    $value = $Response.Headers[$key]
    if (-not $value.ToLowerInvariant().Contains($Needle.ToLowerInvariant())) {
        throw ($Label + ': header ' + $Name + ' did not match (value redacted)')
    }
}

function Assert-ContentContains {
    param($Response, [string]$Needle, [string]$Label)
    if (-not $Response.Content.Contains($Needle)) {
        throw ($Label + ': response body did not match (value redacted)')
    }
}

function Start-IsolatedProcess {
    param([string]$FilePath, [string]$Arguments, [string]$LogPath)
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $FilePath
    $startInfo.Arguments = $Arguments
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($key in @($startInfo.EnvironmentVariables.Keys)) {
        if ($key.StartsWith('VPN233_', [StringComparison]::OrdinalIgnoreCase)) { $startInfo.EnvironmentVariables.Remove($key) }
    }
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $startInfo
    $stdout = New-Object IO.FileStream($LogPath, [IO.FileMode]::Create, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite, 1, [IO.FileOptions]::Asynchronous)
    $stderr = New-Object IO.FileStream(($LogPath + '.err'), [IO.FileMode]::Create, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite, 1, [IO.FileOptions]::Asynchronous)
    try {
        $null = $process.Start()
        $script:LogCopies += [pscustomobject]@{
            OutStream = $stdout
            ErrStream = $stderr
            OutTask = $process.StandardOutput.BaseStream.CopyToAsync($stdout)
            ErrTask = $process.StandardError.BaseStream.CopyToAsync($stderr)
        }
    }
    catch { $stdout.Dispose(); $stderr.Dispose(); throw }
    $script:Processes += $process
    return $process
}

function Start-Vpn233Service {
    param([string]$FilePath, [string]$ConfigPath, [string]$LogPath)
    return Start-IsolatedProcess -FilePath $FilePath -Arguments ('--config "' + $ConfigPath + '"') -LogPath $LogPath
}

function Stop-Vpn233Service {
    param($Process)
    if ($null -eq $Process) { return }
    try {
        if (-not $Process.HasExited) {
            Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue
            $null = $Process.WaitForExit(5000)
        }
    }
    catch {
        # Process already gone; nothing to clean up.
        Write-Verbose ('process already stopped: ' + $_.Exception.Message)
    }
}

function Dump-Logs {
    Write-Host 'Runtime responses and logs remain local; no payload is printed.'
}

function Assert-LogsPrivate {
    param([string[]]$Secrets)
    foreach ($logFile in (Get-ChildItem -LiteralPath $LogDir -File)) {
        $reader = New-Object IO.StreamReader([IO.File]::Open($logFile.FullName, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite))
        try { $logText = $reader.ReadToEnd() }
        finally { $reader.Dispose() }
        foreach ($secret in $Secrets) {
            if ($secret -and $logText.Contains($secret)) { throw ('privacy: a secret leaked into ' + $logFile.Name) }
        }
    }
}

function Build-Qs {
    param([hashtable]$Query)
    $pairs = @()
    foreach ($key in ($Query.Keys | Sort-Object)) {
        $pairs += ([string]$key + '=' + [string]$Query[$key])
    }
    return ($pairs -join '&')
}

# --- test data -----------------------------------------------------------
$SubscribePort = Get-FreePort
$ProviderPort = Get-FreePort
$ClientSocksPort = Get-FreePort
$ClientHttpPort = Get-FreePort
$ClientWebPort = Get-FreePort
$EchoPort = 0

$SubscribeAdminPassword = 'sub-' + (Get-RandomHex -ByteCount 12)
$ProviderAdminPassword = 'prov-' + (Get-RandomHex -ByteCount 12)
$ClientPassword = 'client-' + (Get-RandomHex -ByteCount 12)
$RegistryToken = 'registry-' + (Get-RandomHex -ByteCount 16)
$ProviderFeedToken = 'feed-' + (Get-RandomHex -ByteCount 16)
$SubscribeToken = 'e2etoken' + (Get-RandomHex -ByteCount 10)
$SubscribePassword = 'e2epass' + (Get-RandomHex -ByteCount 10)
$QuotaToken = 'quotatoken' + (Get-RandomHex -ByteCount 10)
$QuotaPassword = 'quotapass' + (Get-RandomHex -ByteCount 10)
$ExpiredToken = 'expiredtoken' + (Get-RandomHex -ByteCount 10)
$ExpiredPassword = 'expiredpass' + (Get-RandomHex -ByteCount 10)

$SubscribeBase = 'http://127.0.0.1:' + $SubscribePort
$ProviderBase = 'http://127.0.0.1:' + $ProviderPort
$ClientWebBase = 'http://127.0.0.1:' + $ClientWebPort
$EchoBase = 'http://127.0.0.1:' + $EchoPort

$VlessLink = 'vless://11111111-1111-4111-8111-111111111111@example.invalid:443?security=tls&type=ws&path=%2Fws&sni=example.invalid&fp=chrome#vless-e2e'
$VmessJson = '{"v":"2","ps":"vmess-e2e","add":"example.invalid","port":"443","id":"22222222-2222-4222-8222-222222222222","aid":"0","scy":"auto","net":"ws","type":"none","host":"example.invalid","path":"/ws","tls":"tls","sni":"example.invalid","fp":"chrome"}'
$VmessLink = 'vmess://' + [Convert]::ToBase64String([System.Text.Encoding]::UTF8.GetBytes($VmessJson))
$TrojanLink = 'trojan://e2e-password@example.invalid:443?sni=example.invalid#trojan-e2e'
$SsLink = 'ss://' + [Convert]::ToBase64String([System.Text.Encoding]::UTF8.GetBytes('aes-256-gcm:e2e-password')) + '@example.invalid:8388#ss-e2e'
$ProviderNodeLink = 'vless://33333333-3333-4333-8333-333333333333@provider.example.invalid:443?security=tls&type=tcp&sni=provider.example.invalid#provider-e2e-node'

$SubscribeConfig = Join-Path $ConfigDir 'subscribe.yaml'
$ProviderConfig = Join-Path $ConfigDir 'provider.yaml'
$ClientConfig = Join-Path $ConfigDir 'client.yaml'

$SubscribeYaml = @"
server:
  host: "127.0.0.1"
  http_port: $SubscribePort
  default_format: "clash"
  data_dir: "$WorkDirUnix/data/subscribe"
  admin_user: "admin"
  admin_password: "$SubscribeAdminPassword"
  admin_origins: "http://127.0.0.1:$SubscribePort"
  web_dir: "$WorkDirUnix/web"
  provider_registry_token: "$RegistryToken"
  provider_stale_after_secs: 120
  client_stats_url: ""
  client_stats_user: ""
  client_stats_password: ""
  traffic_timezone_offset_hours: 8
subscriptions:
  - name: "e2e-static"
    links:
      - "$VlessLink"
      - "$VmessLink"
      - "$TrojanLink"
      - "$SsLink"
    url: ""
    access_token: ""
"@

$ProviderYaml = @"
listen_addr: "127.0.0.1"
listen_port: $ProviderPort
admin_user: "root"
admin_password: "$ProviderAdminPassword"
default_node_ip: "127.0.0.1"
default_port_base: 20000
provider_access_token: "$ProviderFeedToken"
mihomo_url: ""
mihomo_sha256: ""
mihomo_version: "v1.19.0"
subscribe_verify_token: ""
node_links:
  - "$ProviderNodeLink"
registry_url: "$SubscribeBase"
registry_token: "$RegistryToken"
registry_provider_id: "e2e-provider"
registry_name: "e2e-provider"
heartbeat_interval_secs: 10
advertise_addresses:
  - kind: "local"
    value: "127.0.0.1"
enabled_protocols: []
auto_detect_public_ip: false
"@

$ClientYaml = @"
listen_addr: "127.0.0.1"
socks_port: $ClientSocksPort
http_port: $ClientHttpPort
web_addr: "127.0.0.1:$ClientWebPort"
admin_user: "root"
admin_password: "$ClientPassword"
upstream_mode: "direct"
subscription_url: ""
stats_file: "$WorkDirUnix/data/client/stats.json"
stats_timezone_offset_hours: 8
upstream_socks5: ""
socks_user: ""
socks_password: ""
http_user: ""
http_password: ""
record_domains: false
"@

Set-Content -Path $SubscribeConfig -Value $SubscribeYaml -Encoding ASCII
Set-Content -Path $ProviderConfig -Value $ProviderYaml -Encoding ASCII
Set-Content -Path $ClientConfig -Value $ClientYaml -Encoding ASCII

# --- main flow -----------------------------------------------------------
$exitCode = 0
$subscribeProcess = $null
$adminHeaders = @{}

try {
    $providerBinary = Join-Path $BinDir 'vpn233-provider-server.exe'
    $subscribeBinary = Join-Path $BinDir 'vpn233-subscribe-server.exe'
    $clientBinary = Join-Path $BinDir 'vpn233-client.exe'
    foreach ($binary in @($providerBinary, $subscribeBinary, $clientBinary)) {
        if (-not (Test-Path $binary)) {
            throw ('missing binary ' + $binary + '; run cargo build --release --workspace first')
        }
    }

    Step-Begin 'start services'
    $subscribeProcess = Start-Vpn233Service -FilePath $subscribeBinary -ConfigPath $SubscribeConfig -LogPath (Join-Path $LogDir 'subscribe.log')
    $null = Start-Vpn233Service -FilePath $providerBinary -ConfigPath $ProviderConfig -LogPath (Join-Path $LogDir 'provider.log')
    $null = Start-Vpn233Service -FilePath $clientBinary -ConfigPath $ClientConfig -LogPath (Join-Path $LogDir 'client.log')
    if (-not (Wait-HttpReady -Url ($SubscribeBase + '/api/v1/formats') -Seconds $TimeoutSeconds)) { throw 'subscribe-server did not become ready' }
    if (-not (Wait-HttpReady -Url ($ProviderBase + '/api/v1/health') -Seconds $TimeoutSeconds)) { throw 'provider-server did not become ready' }
    if (-not (Wait-HttpReady -Url ($ClientWebBase + '/api/v1/health') -Seconds $TimeoutSeconds)) { throw 'vpn233-client did not become ready' }
    $webResponse = Invoke-Api -Method 'GET' -Url ($SubscribeBase + '/')
    Assert-Status -Response $webResponse -Expected 200 -Label 'generated admin UI'
    Assert-ContentContains -Response $webResponse -Needle 'id="root"' -Label 'generated admin UI root'
    Write-Note ('subscribe=' + $SubscribeBase + ' provider=' + $ProviderBase + ' client=' + $ClientWebBase)
    Step-End

    Step-Begin 'admin login and access token'
    $loginBody = @{ username = 'admin'; password = $SubscribeAdminPassword } | ConvertTo-Json -Compress
    $login = Invoke-Api -Method 'POST' -Url ($SubscribeBase + '/api/auth/login') -Body $loginBody
    Assert-Status -Response $login -Expected 200 -Label 'admin login'
    $adminJwt = [string]($login.Content | ConvertFrom-Json).token
    if ([string]::IsNullOrEmpty($adminJwt)) { throw 'admin login returned an empty token' }
    $adminHeaders = @{ Authorization = 'Bearer ' + $adminJwt }

    $status = Invoke-Api -Method 'GET' -Url ($SubscribeBase + '/api/status') -Headers $adminHeaders
    Assert-Status -Response $status -Expected 200 -Label 'authenticated /api/status'
    $statusAnon = Invoke-Api -Method 'GET' -Url ($SubscribeBase + '/api/status')
    Assert-Status -Response $statusAnon -Expected 401 -Label 'unauthenticated /api/status'
    Write-Note 'admin JWT acquired'
    Step-End

    Step-Begin 'capability endpoints'
    $formats = Invoke-Api -Method 'GET' -Url ($SubscribeBase + '/api/v1/formats')
    Assert-Status -Response $formats -Expected 200 -Label '/api/v1/formats'
    Assert-HeaderContains -Response $formats -Name 'Content-Type' -Needle 'application/json' -Label '/api/v1/formats'
    $formatsJson = $formats.Content | ConvertFrom-Json
    if (@($formatsJson.formats).Count -lt 9) { throw '/api/v1/formats: expected at least 9 formats' }
    if (-not (@($formatsJson.protocols) -contains 'vless')) { throw '/api/v1/formats: vless missing from protocols' }

    $providerLoginBody = @{ username = 'root'; password = $ProviderAdminPassword } | ConvertTo-Json -Compress
    $providerLogin = Invoke-Api -Method 'POST' -Url ($ProviderBase + '/api/v1/login') -Body $providerLoginBody
    Assert-Status -Response $providerLogin -Expected 200 -Label 'provider login'
    $providerJwt = [string]($providerLogin.Content | ConvertFrom-Json).token
    $providerHeaders = @{ Authorization = 'Bearer ' + $providerJwt }
    $generated = Invoke-Api -Method 'GET' -Url ($ProviderBase + '/api/v1/generate?format=json') -Headers $providerHeaders
    Assert-Status -Response $generated -Expected 200 -Label 'provider generate json'
    $manifest = $generated.Content | ConvertFrom-Json
    if (@($manifest.node.ports).Count -lt 3) { throw 'provider generate: fewer than 3 node ports' }
    $generateSh = Invoke-Api -Method 'GET' -Url ($ProviderBase + '/api/v1/generate.sh') -Headers $providerHeaders
    Assert-Status -Response $generateSh -Expected 200 -Label 'provider generate.sh'
    Assert-ContentContains -Response $generateSh -Needle '#!/usr/bin/env bash' -Label 'provider generate.sh'
    $generatePs1 = Invoke-Api -Method 'GET' -Url ($ProviderBase + '/api/v1/generate.ps1') -Headers $providerHeaders
    Assert-Status -Response $generatePs1 -Expected 200 -Label 'provider generate.ps1'
    Assert-ContentContains -Response $generatePs1 -Needle 'Set-StrictMode' -Label 'provider generate.ps1'
    $feed = Invoke-Api -Method 'GET' -Url ($ProviderBase + '/api/v1/nodes') -Headers @{ 'x-provider-token' = $ProviderFeedToken }
    Assert-Status -Response $feed -Expected 200 -Label 'provider nodes feed'
    Assert-ContentContains -Response $feed -Needle 'provider.example.invalid' -Label 'provider nodes feed'
    Write-Note 'provider capability and manifest endpoints verified'
    Step-End

    Step-Begin 'create account and subscription link'
    $accountBody = @{ username = 'e2e-user'; display_name = 'E2E user' } | ConvertTo-Json -Compress
    $account = Invoke-Api -Method 'POST' -Url ($SubscribeBase + '/api/admin/accounts') -Headers $adminHeaders -Body $accountBody
    Assert-Status -Response $account -Expected 201 -Label 'create account'
    $accountId = [string]($account.Content | ConvertFrom-Json).account.account_id
    if ([string]::IsNullOrEmpty($accountId)) { throw 'create account: empty account_id' }

    $linkBody = @{ token = $SubscribeToken; password = $SubscribePassword; limit_bytes = 104857600; label = 'e2e-primary' } | ConvertTo-Json -Compress
    $link = Invoke-Api -Method 'POST' -Url ($SubscribeBase + '/api/admin/accounts/' + $accountId + '/links') -Headers $adminHeaders -Body $linkBody
    Assert-Status -Response $link -Expected 201 -Label 'create account link'

    $access = Invoke-Api -Method 'POST' -Url ($SubscribeBase + '/api/admin/tokens/' + $SubscribeToken + '/link') -Headers $adminHeaders
    Assert-Status -Response $access -Expected 200 -Label 'create subscription access token'
    $subscribeAccess = [string]($access.Content | ConvertFrom-Json).access_token
    if ([string]::IsNullOrEmpty($subscribeAccess)) { throw 'subscription access token is empty' }
    Write-Note 'account, link and access token created'
    Step-End

    Step-Begin 'subscription formats'
    $formatList = @('clash', 'singbox', 'v2ray', 'links', 'surge', 'quantumultx', 'loon', 'surfboard', 'stash')
    foreach ($format in $formatList) {
        $expectedType = 'text/plain'
        $extension = 'txt'
        $signature = '://'
        $marker = 'vless://'
        if ($format -eq 'clash' -or $format -eq 'stash') { $expectedType = 'text/yaml'; $extension = 'yaml'; $signature = 'proxies:'; $marker = 'type: vless' }
        if ($format -eq 'singbox') { $expectedType = 'application/json'; $extension = 'json'; $signature = 'outbounds'; $marker = '"type": "vless"' }
        if ($format -eq 'surge' -or $format -eq 'surfboard') { $expectedType = 'text/plain'; $extension = 'conf'; $signature = '[Proxy]'; $marker = '= vmess,' }
        if ($format -eq 'quantumultx') { $expectedType = 'text/plain'; $extension = 'conf'; $signature = '[server_local]'; $marker = 'vless=' }
        if ($format -eq 'loon') { $expectedType = 'text/plain'; $extension = 'conf'; $signature = '[Proxy]'; $marker = '= VLESS,' }

        $query = Build-Qs -Query @{ token = $SubscribeToken; access_token = $subscribeAccess; filename = ('vpn233-' + $format + '.' + $extension) }
        $rendered = Invoke-Api -Method 'GET' -Url ($SubscribeBase + '/sub/' + $format + '?' + $query)
        Assert-Status -Response $rendered -Expected 200 -Label ('format ' + $format)
        Assert-HeaderContains -Response $rendered -Name 'Content-Type' -Needle $expectedType -Label ('format ' + $format)
        Assert-HeaderContains -Response $rendered -Name 'X-VPN233-Format' -Needle $format -Label ('format ' + $format)
        Assert-HeaderContains -Response $rendered -Name 'Content-Disposition' -Needle ('.' + $extension) -Label ('format ' + $format)
        Assert-HeaderContains -Response $rendered -Name 'subscription-userinfo' -Needle 'total=' -Label ('format ' + $format)
        Assert-HeaderContains -Response $rendered -Name 'Cache-Control' -Needle 'no-store' -Label ('format ' + $format)
        $nodesHeader = [string]$rendered.Headers['x-vpn233-nodes']
        $nodesValue = 0
        if (-not [int]::TryParse($nodesHeader, [ref]$nodesValue)) { throw ('format ' + $format + ': X-VPN233-Nodes is not an integer') }
        if ($nodesValue -lt 1) { throw ('format ' + $format + ': rendered zero nodes') }
        if (-not $rendered.Headers.ContainsKey('x-vpn233-skipped')) { throw ('format ' + $format + ': missing X-VPN233-Skipped') }

        $assertionContent = $rendered.Content
        if ($format -eq 'singbox') {
            $null = $rendered.Content | ConvertFrom-Json
            Assert-ContentContains -Response $rendered -Needle '"outbounds"' -Label 'format singbox'
        }
        elseif ($format -eq 'v2ray') {
            try {
                $decoded = [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($rendered.Content.Trim()))
            }
            catch {
                throw 'format v2ray: body is not valid base64'
            }
            if (-not $decoded.Contains('://')) { throw 'format v2ray: decoded body has no proxy links' }
            $decodedLines = @($decoded -split '\r?\n' | Where-Object { $_.Trim() })
            if ($decodedLines.Count -ne $nodesValue) { throw 'format v2ray: decoded link count does not match the node header' }
            foreach ($decodedLine in $decodedLines) {
                if ($decodedLine -notmatch '^(vless|vmess|trojan|ss|ssr|hysteria2|hy2|tuic|anytls|socks5|socks|http|https)://') { throw 'format v2ray: invalid decoded proxy scheme (value redacted)' }
            }
            $assertionContent = $decoded
        }
        if (-not $assertionContent.Contains($signature)) { throw ('format ' + $format + ': syntax signature missing (content redacted)') }
        if (-not $assertionContent.Contains($marker)) { throw ('format ' + $format + ': protocol syntax marker missing (content redacted)') }
        Write-Note ('format ' + $format + ': nodes=' + $nodesValue + ', bytes=' + $rendered.Content.Length)
    }

    $defaultQuery = Build-Qs -Query @{ token = $SubscribeToken; access_token = $subscribeAccess }
    $defaultSub = Invoke-Api -Method 'GET' -Url ($SubscribeBase + '/sub?' + $defaultQuery)
    Assert-Status -Response $defaultSub -Expected 200 -Label 'default subscription endpoint'
    Assert-HeaderContains -Response $defaultSub -Name 'X-VPN233-Format' -Needle 'clash' -Label 'default subscription format'
    $override = Invoke-Api -Method 'GET' -Url ($SubscribeBase + '/sub/c?' + $defaultQuery + '&format=singbox')
    Assert-Status -Response $override -Expected 200 -Label 'format override on /sub/c'
    Assert-HeaderContains -Response $override -Name 'X-VPN233-Format' -Needle 'singbox' -Label 'format override on /sub/c'
    $invalidFormat = Invoke-Api -Method 'GET' -Url ($SubscribeBase + '/sub/clash?' + $defaultQuery + '&format=not-a-format')
    Assert-Status -Response $invalidFormat -Expected 400 -Label 'invalid format override'
    $anonymous = Invoke-Api -Method 'GET' -Url ($SubscribeBase + '/sub/clash')
    Assert-Status -Response $anonymous -Expected 401 -Label 'subscription without credentials'
    $wrongPassword = Invoke-Api -Method 'GET' -Url ($SubscribeBase + '/sub/clash?token=' + $SubscribeToken + '&password=wrong-password')
    Assert-Status -Response $wrongPassword -Expected 401 -Label 'subscription with wrong password'
    Step-End

    Step-Begin 'quota enforcement'
    $quotaBody = @{ token = $QuotaToken; password = $QuotaPassword; limit_bytes = 1; label = 'e2e-quota' } | ConvertTo-Json -Compress
    $quotaLink = Invoke-Api -Method 'POST' -Url ($SubscribeBase + '/api/admin/accounts/' + $accountId + '/links') -Headers $adminHeaders -Body $quotaBody
    Assert-Status -Response $quotaLink -Expected 201 -Label 'create quota link'
    $quotaAccessResponse = Invoke-Api -Method 'POST' -Url ($SubscribeBase + '/api/admin/tokens/' + $QuotaToken + '/link') -Headers $adminHeaders
    Assert-Status -Response $quotaAccessResponse -Expected 200 -Label 'quota access token'
    $quotaAccess = [string]($quotaAccessResponse.Content | ConvertFrom-Json).access_token
    $quotaHit = $false
    $attempt = 0
    while ($attempt -lt 5 -and -not $quotaHit) {
        $attempt = $attempt + 1
        $quotaResponse = Invoke-Api -Method 'GET' -Url ($SubscribeBase + '/sub/links?token=' + $QuotaToken + '&access_token=' + $quotaAccess)
        if ($quotaResponse.Status -eq 429) { $quotaHit = $true }
    }
    if (-not $quotaHit) { throw 'quota link never returned HTTP 429' }
    Write-Note ('quota exceeded after ' + $attempt + ' request(s)')
    Step-End

    Step-Begin 'expired token'
    $past = [long](([DateTimeOffset]::UtcNow.ToUnixTimeSeconds()) - 3600)
    $expiredBody = @{ token = $ExpiredToken; password = $ExpiredPassword; limit_bytes = 1048576; expires_at = $past; label = 'e2e-expired' } | ConvertTo-Json -Compress
    $expiredLink = Invoke-Api -Method 'POST' -Url ($SubscribeBase + '/api/admin/accounts/' + $accountId + '/links') -Headers $adminHeaders -Body $expiredBody
    Assert-Status -Response $expiredLink -Expected 201 -Label 'create expired link'
    $expired = Invoke-Api -Method 'GET' -Url ($SubscribeBase + '/sub/links?token=' + $ExpiredToken + '&password=' + $ExpiredPassword)
    Assert-Status -Response $expired -Expected 401 -Label 'expired token'
    Step-End

    Step-Begin 'provider registry'
    $registryOnline = $false
    $deadline = (Get-Date).AddSeconds(40)
    while ((Get-Date) -lt $deadline -and -not $registryOnline) {
        $providers = Invoke-Api -Method 'GET' -Url ($SubscribeBase + '/api/admin/providers') -Headers $adminHeaders
        if ($providers.Status -eq 200) {
            $providersJson = $providers.Content | ConvertFrom-Json
            foreach ($provider in @($providersJson.providers)) {
                if ($provider.provider_id -eq 'e2e-provider' -and $provider.online -eq $true -and [int]$provider.node_count -gt 0) {
                    $registryOnline = $true
                }
            }
        }
        if (-not $registryOnline) { Start-Sleep -Seconds 1 }
    }
    if (-not $registryOnline) { throw 'provider never appeared online in /api/admin/providers' }
    Write-Note 'provider registered and online'
    Step-End

    Step-Begin 'registered nodes appear in subscriptions'
    $found = $false
    $deadline = (Get-Date).AddSeconds(20)
    while ((Get-Date) -lt $deadline -and -not $found) {
        $linksResponse = Invoke-Api -Method 'GET' -Url ($SubscribeBase + '/sub/links?' + $defaultQuery)
        if ($linksResponse.Status -eq 200 -and $linksResponse.Content.Contains('provider.example.invalid')) {
            $found = $true
        }
        else {
            Start-Sleep -Seconds 1
        }
    }
    if (-not $found) { throw 'registered provider node did not appear in the subscription' }
    Write-Note 'provider node visible in /sub/links'
    Step-End

    Step-Begin 'client proxy (SOCKS5 and HTTP)'
    $echoReady = Join-Path $WorkDir 'echo.ready'
    $echoProcess = Start-IsolatedProcess -FilePath $XtaskBinary `
        -Arguments ('network echo --port 0 --ready-file "' + $echoReady + '" --marker vpn233-e2e-echo') `
        -LogPath (Join-Path $LogDir 'echo.log')
    $echoDeadline = (Get-Date).AddSeconds(15)
    while ((Get-Date) -lt $echoDeadline) {
        if ($echoProcess.HasExited) { throw 'Rust echo fixture exited before readiness (private log retained locally)' }
        if (Test-Path -LiteralPath $echoReady -PathType Leaf) {
            $readyText = [System.IO.File]::ReadAllText($echoReady).Trim()
            if ($readyText -match '^[1-9][0-9]{0,4}$' -and [int]$readyText -le 65535) { $EchoPort = [int]$readyText; break }
        }
        Start-Sleep -Milliseconds 50
    }
    if ($EchoPort -eq 0) { throw 'Rust echo fixture readiness timed out' }
    $EchoBase = 'http://127.0.0.1:' + $EchoPort
    if (-not (Wait-HttpReady -Url ($EchoBase + '/') -Seconds 5)) { throw 'Rust echo server did not serve HTTP' }

    $curl = Get-Command -Name 'curl.exe' -ErrorAction SilentlyContinue
    if ($null -eq $curl) { throw 'curl.exe is required for the Windows E2E proxy checks' }
    $noProxyArgument = Get-CurlNoProxyArgument
    $previousNoProxy = $env:NO_PROXY
    try {
        # Exercise the real proxy even under a user's broad inherited bypass.
        $env:NO_PROXY = '*'
        $socksOut = & $curl.Source '--silent' '--show-error' '--max-time' '20' '--noproxy' $noProxyArgument '--socks5-hostname' ('127.0.0.1:' + $ClientSocksPort) ($EchoBase + '/echo/socks')
        if ($LASTEXITCODE -ne 0) { throw 'SOCKS5 proxy request failed' }
        if (-not $socksOut.Contains('/echo/socks')) { throw 'SOCKS5 proxy returned unexpected body (redacted)' }
        Write-Note 'SOCKS5 proxy request reached the echo server'

        $httpOut = & $curl.Source '--silent' '--show-error' '--max-time' '20' '--noproxy' $noProxyArgument '-x' ('http://127.0.0.1:' + $ClientHttpPort) ($EchoBase + '/echo/http')
        if ($LASTEXITCODE -ne 0) { throw 'HTTP proxy request failed' }
        if (-not $httpOut.Contains('/echo/http')) { throw 'HTTP proxy returned unexpected body (redacted)' }
        Write-Note 'HTTP proxy request reached the echo server'
    } finally { $env:NO_PROXY = $previousNoProxy }
    Step-End

    Step-Begin 'client stats and privacy endpoints'
    $basicAuth = [Convert]::ToBase64String([System.Text.Encoding]::UTF8.GetBytes('root:' + $ClientPassword))
    $stats = Invoke-Api -Method 'GET' -Url ($ClientWebBase + '/api/v1/stats') -Headers @{ Authorization = 'Basic ' + $basicAuth }
    Assert-Status -Response $stats -Expected 200 -Label 'client stats'
    $statsJson = $stats.Content | ConvertFrom-Json
    if ([int]$statsJson.total_connections -lt 2) { throw ('client stats: total_connections=' + $statsJson.total_connections + ', expected at least 2') }
    $domainCount = @($statsJson.domains.PSObject.Properties).Count
    if ($domainCount -ne 0) { throw ('client stats: record_domains=false but ' + $domainCount + ' domain(s) were recorded') }
    $statsAnon = Invoke-Api -Method 'GET' -Url ($ClientWebBase + '/api/v1/stats')
    Assert-Status -Response $statsAnon -Expected 401 -Label 'client stats without credentials'
    $clientConfigView = Invoke-Api -Method 'GET' -Url ($ClientWebBase + '/api/v1/config') -Headers @{ Authorization = 'Basic ' + $basicAuth }
    Assert-Status -Response $clientConfigView -Expected 200 -Label 'client config endpoint'
    if ($clientConfigView.Content.Contains($ClientPassword)) { throw 'client config endpoint leaked the admin password' }
    $pac = Invoke-Api -Method 'GET' -Url ($ClientWebBase + '/proxy.pac')
    Assert-Status -Response $pac -Expected 200 -Label 'client PAC endpoint'
    Assert-HeaderContains -Response $pac -Name 'Content-Type' -Needle 'application/x-ns-proxy-autoconfig' -Label 'client PAC endpoint'
    Write-Note ('total_connections=' + $statsJson.total_connections + ', domains=0')
    Step-End

    Step-Begin 'privacy checks (logs)'
    $secrets = @($SubscribeAdminPassword, $ProviderAdminPassword, $ClientPassword, $subscribeAccess, $RegistryToken, $ProviderFeedToken, $SubscribeToken, $QuotaToken, $adminJwt, $providerJwt, $quotaAccess)
    Assert-LogsPrivate -Secrets $secrets
    Write-Note 'no credentials, access tokens or feed tokens in service logs'
    Write-Note 'data-file permission check is Linux-only (see scripts/e2e/e2e.sh)'
    Step-End

    Step-Begin 'persistence across subscribe-server restart'
    Stop-Vpn233Service -Process $subscribeProcess
    Start-Sleep -Milliseconds 500
    $subscribeProcess = Start-Vpn233Service -FilePath $subscribeBinary -ConfigPath $SubscribeConfig -LogPath (Join-Path $LogDir 'subscribe-restart.log')
    if (-not (Wait-HttpReady -Url ($SubscribeBase + '/api/v1/formats') -Seconds $TimeoutSeconds)) { throw 'subscribe-server did not restart' }
    $afterRestart = Invoke-Api -Method 'GET' -Url ($SubscribeBase + '/sub/links?' + $defaultQuery)
    Assert-Status -Response $afterRestart -Expected 200 -Label 'token after subscribe restart'
    Assert-ContentContains -Response $afterRestart -Needle 'example.invalid' -Label 'subscription after restart'
    $loginAgain = Invoke-Api -Method 'POST' -Url ($SubscribeBase + '/api/auth/login') -Body $loginBody
    Assert-Status -Response $loginAgain -Expected 200 -Label 'admin login after restart'
    Write-Note 'token and admin credentials survived the restart'
    Step-End
    Step-Begin 'privacy across restart (stdout and stderr)'
    $restartJwt = [string]($loginAgain.Content | ConvertFrom-Json).token
    Assert-LogsPrivate -Secrets ($secrets + @($restartJwt))
    Step-End

    Write-Host ''
    Write-Host '============================================' -ForegroundColor Green
    Write-Host 'E2E PASSED' -ForegroundColor Green
    Write-Host '============================================' -ForegroundColor Green
    $totalMilliseconds = 0
    $index = 0
    while ($index -lt $script:StepNames.Count) {
        $name = $script:StepNames[$index].PadRight(44)
        Write-Host ('  ' + $name + ' ' + $script:StepMilliseconds[$index].ToString().PadLeft(8) + ' ms')
        $totalMilliseconds = $totalMilliseconds + $script:StepMilliseconds[$index]
        $index = $index + 1
    }
    $totalName = 'total'.PadRight(44)
    Write-Host ('  ' + $totalName + ' ' + $totalMilliseconds.ToString().PadLeft(8) + ' ms')
    Write-Host '============================================' -ForegroundColor Green
}
catch {
    $exitCode = 1
    Write-Host ''
    Write-Host ('E2E FAILED: ' + $_.Exception.Message) -ForegroundColor Red
    Dump-Logs
}
finally {
    foreach ($process in $script:Processes) {
        Stop-Vpn233Service -Process $process
    }
    foreach ($copy in $script:LogCopies) {
        $copy.OutTask.Wait()
        $copy.ErrTask.Wait()
        $copy.OutStream.Dispose()
        $copy.ErrStream.Dispose()
    }
    if ($exitCode -ne 0 -or $Keep.IsPresent) {
        Write-Host ('E2E working directory kept for diagnosis: ' + $WorkDir) -ForegroundColor Yellow
    }
    else {
        $resolvedWorkDir = [System.IO.Path]::GetFullPath($WorkDir)
        $resolvedTempDir = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
        if (-not $resolvedWorkDir.StartsWith($resolvedTempDir, [StringComparison]::OrdinalIgnoreCase)) { throw 'unsafe E2E cleanup path' }
        Remove-Item -LiteralPath $resolvedWorkDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

exit $exitCode
