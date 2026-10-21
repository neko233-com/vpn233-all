# Shared GitHub download contract. Keep the anchors identical to mirrors.sh.
$script:VPN233_GH_MIRROR_PREFIXES_DEFAULT = 'https://ghproxy.net/ https://gh.llkk.cc/ https://ghfast.top/ https://gh-proxy.com/ '
$script:VPN233_GH_RAW_PREFIXES_DEFAULT = 'https://ghproxy.net/ https://gh.llkk.cc/ https://ghfast.top/ https://gh-proxy.com/ '
$script:VPN233_LAST_MIRROR = ''

function ConvertTo-Vpn233MirrorPrefix {
    param([AllowEmptyString()][string]$Prefix = '')
    if (-not $Prefix) { return '' }
    $uri = $null
    if (-not [uri]::TryCreate($Prefix, [UriKind]::Absolute, [ref]$uri) -or
        $uri.Scheme -notin @('https', 'http') -or -not $uri.Host -or
        $uri.UserInfo -or $uri.Query -or $uri.Fragment -or $Prefix -match '\s') {
        throw 'Mirror must be an HTTP(S) prefix without credentials, query, fragment or whitespace.'
    }
    return $Prefix.TrimEnd('/') + '/'
}

function Set-Vpn233MirrorMode {
    param([string]$Mode = 'auto')
    switch ($Mode.ToLowerInvariant()) {
        'auto' { $env:VPN233_MIRROR_MODE = $null; return }
        { $_ -in @('off', 'none', 'direct') } { $env:VPN233_MIRROR_MODE = 'off'; return }
        default { $env:VPN233_MIRROR_MODE = ConvertTo-Vpn233MirrorPrefix $Mode }
    }
}

function Get-Vpn233MirrorPrefixes {
    param([ValidateSet('release', 'raw')][string]$Kind = 'release')
    $mode = $env:VPN233_MIRROR_MODE
    if ($env:VPN233_NO_MIRROR -match '^(1|true|yes)$' -or $mode -in @('off', 'none', 'direct')) {
        Write-Output ''; return
    }
    if ($mode -and $mode -ne 'auto') { ConvertTo-Vpn233MirrorPrefix $mode; return }
    if ($env:VPN233_GH_MIRRORS) {
        foreach ($prefix in $env:VPN233_GH_MIRRORS.Split('|')) { ConvertTo-Vpn233MirrorPrefix $prefix }
        return
    }
    if ($env:VPN233_GH_MIRROR) { ConvertTo-Vpn233MirrorPrefix $env:VPN233_GH_MIRROR; return }
    $contract = if ($Kind -eq 'raw') { $script:VPN233_GH_RAW_PREFIXES_DEFAULT } else { $script:VPN233_GH_MIRROR_PREFIXES_DEFAULT }
    foreach ($prefix in $contract.Trim().Split(' ')) { Write-Output $prefix }
    Write-Output ''
}

function Get-Vpn233MirrorCachePath {
    $baseDir = if ($env:VPN233_HOME) { $env:VPN233_HOME } elseif ($env:USERPROFILE) { Join-Path $env:USERPROFILE '.vpn233' } else { Join-Path ([IO.Path]::GetTempPath()) 'vpn233' }
    return Join-Path $baseDir 'mirror-cache'
}

function Get-Vpn233OrderedMirrorPrefixes {
    param([ValidateSet('release', 'raw')][string]$Kind = 'release')
    $prefixes = @(Get-Vpn233MirrorPrefixes $Kind)
    $cache = Get-Vpn233MirrorCachePath
    if (Test-Path -LiteralPath $cache -PathType Leaf) {
        $line = Get-Content -LiteralPath $cache | Where-Object { $_.StartsWith($Kind + ' ') } | Select-Object -First 1
        if ($null -ne $line) {
            $cached = $line.Substring($Kind.Length + 1)
            if ($prefixes -contains $cached) {
                Write-Output $cached
                $prefixes | Where-Object { $_ -ne $cached }
                return
            }
        }
    }
    $prefixes | Select-Object -Unique
}

function Save-Vpn233MirrorCache {
    param([string]$Kind, [AllowEmptyString()][string]$Prefix)
    # Cache is optional. Its failure cannot turn a verified download into failure.
    $temporary = ''
    try {
        $cache = Get-Vpn233MirrorCachePath
        $directory = Split-Path -Parent $cache
        New-Item -ItemType Directory -Force -Path $directory | Out-Null
        $lines = @()
        if (Test-Path -LiteralPath $cache -PathType Leaf) {
            $lines = @(Get-Content -LiteralPath $cache | Where-Object { $_ -and -not $_.StartsWith($Kind + ' ') })
        }
        $temporary = $cache + '.' + [guid]::NewGuid().ToString('N') + '.tmp'
        [IO.File]::WriteAllLines($temporary, [string[]]@($lines + ($Kind + ' ' + $Prefix)))
        Move-Item -LiteralPath $temporary -Destination $cache -Force
    } catch {
        if ($temporary -and (Test-Path -LiteralPath $temporary)) { Remove-Item -LiteralPath $temporary -Force }
    }
}

function Get-Vpn233Version {
    param([string]$Version)
    $normalized = $Version -replace '^[vV]', ''
    if ($normalized -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$') { throw 'Version must be a semantic version such as 2.1.0.' }
    return $normalized
}

function Get-Vpn233ReleaseUrl {
    param([string]$Asset, [string]$Version = '')
    if ($Asset -notmatch '^[A-Za-z0-9._-]+$') { throw 'Invalid release asset name.' }
    $owner = if ($env:VPN233_REPO_OWNER) { $env:VPN233_REPO_OWNER } else { 'neko233-com' }
    $repo = if ($env:VPN233_REPO_NAME) { $env:VPN233_REPO_NAME } else { 'vpn233-all' }
    if ($owner -notmatch '^[A-Za-z0-9_-]+$' -or $repo -notmatch '^[A-Za-z0-9._-]+$') { throw 'Invalid GitHub repository.' }
    $releasePath = if ($Version) { 'download/v' + (Get-Vpn233Version $Version) } else { 'latest/download' }
    return "https://github.com/$owner/$repo/releases/$releasePath/$Asset"
}

function Invoke-Vpn233Fetch {
    param([string]$Url, [string]$Destination)
    $timeout = 300
    if ($env:VPN233_GH_MAX_TIME) {
        $timeout = 0
        if (-not [int]::TryParse($env:VPN233_GH_MAX_TIME, [ref]$timeout) -or $timeout -lt 1) { throw 'VPN233_GH_MAX_TIME must be a positive number of seconds.' }
    }
    # Do not include URL/exception text: custom prefixes must never leak secrets.
    Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $Destination -TimeoutSec $timeout -ErrorAction Stop | Out-Null
}

function Invoke-Vpn233AtomicDownload {
    param([string]$Url, [string]$Destination)
    $temporary = $Destination + '.' + [guid]::NewGuid().ToString('N') + '.part'
    try {
        Invoke-Vpn233Fetch $Url $temporary
        if (-not (Test-Path -LiteralPath $temporary -PathType Leaf) -or (Get-Item -LiteralPath $temporary).Length -eq 0) { return $false }
        Move-Item -LiteralPath $temporary -Destination $Destination -Force
        return $true
    } catch { return $false }
    finally { if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force } }
}

function Receive-Vpn233ReleaseAsset {
    param([string]$Asset, [string]$Destination, [string]$Version = '', [switch]$Quiet)
    $canonical = Get-Vpn233ReleaseUrl $Asset $Version
    foreach ($prefix in @(Get-Vpn233OrderedMirrorPrefixes release)) {
        if (Invoke-Vpn233AtomicDownload ($prefix + $canonical) $Destination) {
            $script:VPN233_LAST_MIRROR = $prefix
            Save-Vpn233MirrorCache release $prefix
            if (-not $Quiet) { Write-Host ('  Downloaded ' + $Asset) }
            return
        }
        Write-Warning 'GitHub source unavailable; trying next configured source.'
    }
    throw ('Unable to download ' + $Asset + ' from any configured GitHub source.')
}

function Receive-Vpn233RawFile {
    param([string]$Path, [string]$Destination, [switch]$Quiet)
    if ($Path -notmatch '^[A-Za-z0-9._/-]+$' -or $Path -match '(^|/)\.\.(/|$)') { throw 'Invalid repository path.' }
    $owner = if ($env:VPN233_REPO_OWNER) { $env:VPN233_REPO_OWNER } else { 'neko233-com' }
    $repo = if ($env:VPN233_REPO_NAME) { $env:VPN233_REPO_NAME } else { 'vpn233-all' }
    $ref = if ($env:VPN233_REPO_REF) { $env:VPN233_REPO_REF } else { 'main' }
    if ($owner -notmatch '^[A-Za-z0-9_-]+$' -or $repo -notmatch '^[A-Za-z0-9._-]+$' -or $ref -notmatch '^[A-Za-z0-9._/-]+$') { throw 'Invalid GitHub repository or ref.' }
    $canonical = "https://raw.githubusercontent.com/$owner/$repo/$ref/$($Path.TrimStart('/'))"
    foreach ($prefix in @(Get-Vpn233OrderedMirrorPrefixes raw)) {
        if (Invoke-Vpn233AtomicDownload ($prefix + $canonical) $Destination) {
            $script:VPN233_LAST_MIRROR = $prefix
            Save-Vpn233MirrorCache raw $prefix
            if (-not $Quiet) { Write-Host ('  Downloaded ' + $Path) }
            return
        }
        Write-Warning 'GitHub source unavailable; trying next configured source.'
    }
    throw ('Unable to download ' + $Path + ' from any configured GitHub source.')
}

function Get-Vpn233Checksum {
    param([string]$Sums, [string]$Name)
    $matchesFound = @()
    foreach ($line in Get-Content -LiteralPath $Sums) {
        if ($line -match '^(\S+)[ \t]+\*?(.+?)\s*$' -and $Matches[2] -ceq $Name) {
            $digest = $Matches[1]
            if ($digest -notmatch '^[0-9a-fA-F]{64}$') { throw 'SHA256SUMS contains a malformed checksum for the requested asset.' }
            $matchesFound += $digest.ToLowerInvariant()
        }
    }
    if ($matchesFound.Count -ne 1) { throw ('SHA256SUMS must contain exactly one valid checksum for ' + $Name) }
    return $matchesFound[0]
}

function Test-Vpn233Checksum {
    param([string]$Path, [string]$Expected)
    if ($Expected -notmatch '^[0-9a-fA-F]{64}$') { throw 'Expected SHA-256 must contain exactly 64 hexadecimal characters.' }
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash -ine $Expected) { throw 'SHA-256 verification failed; the current installation was preserved.' }
}
