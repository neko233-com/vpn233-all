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

function Invoke-Vpn233RangeFetch {
    param([string]$Url, [string]$Path, [long]$KnownTotal, [int]$BudgetMilliseconds)
    $result = [pscustomobject]@{ Resume = $false; Total = $KnownTotal }
    $request = $null; $response = $null; $inputStream = $null; $outputStream = $null
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $previousTls = [Net.ServicePointManager]::SecurityProtocol
    [long]$maximumBytes = 1GB
    try {
        $offset = (Get-Item -LiteralPath $Path).Length
        if ($offset -gt $maximumBytes -or $BudgetMilliseconds -lt 1) { return $result }
        [Net.ServicePointManager]::SecurityProtocol = $previousTls -bor [Net.SecurityProtocolType]::Tls12
        $request = [Net.HttpWebRequest]::Create($Url)
        $request.Method = 'GET'; $request.KeepAlive = $false
        $request.AutomaticDecompression = [Net.DecompressionMethods]::None
        $request.Headers['Accept-Encoding'] = 'identity'
        $request.Timeout = $BudgetMilliseconds; $request.ReadWriteTimeout = $BudgetMilliseconds
        $request.MaximumAutomaticRedirections = 5
        if ($offset -gt 0) { $request.AddRange([long]$offset) }
        $responseTask = $request.GetResponseAsync()
        $remaining = [int][Math]::Max(0, $BudgetMilliseconds - $clock.ElapsedMilliseconds)
        if ($remaining -lt 1 -or -not $responseTask.Wait($remaining)) { return $result }
        $response = $responseTask.GetAwaiter().GetResult()
        if (([uri]$Url).Scheme -eq 'https' -and $response.ResponseUri.Scheme -ne 'https') { return $result }
        if ($response.ContentEncoding -and $response.ContentEncoding -ne 'identity') { return $result }
        [long]$bodyLength = $response.ContentLength
        if ([int]$response.StatusCode -eq 200) {
            # A server ignoring Range sent a new entity. Never append it.
            $offset = 0
            $result.Total = $bodyLength
        } elseif ([int]$response.StatusCode -eq 206) {
            $range = [string]$response.Headers['Content-Range']
            if ($range -notmatch '^bytes ([0-9]+)-([0-9]+)/([0-9]+)$') { return $result }
            [long]$first = 0; [long]$last = 0; [long]$total = 0
            if (-not [long]::TryParse($Matches[1], [ref]$first) -or
                -not [long]::TryParse($Matches[2], [ref]$last) -or
                -not [long]::TryParse($Matches[3], [ref]$total) -or
                $first -ne $offset -or $last -lt $first -or $total -le $last -or
                $total -gt $maximumBytes -or ($KnownTotal -ge 0 -and $total -ne $KnownTotal)) { return $result }
            $span = $last - $first + 1
            if ($bodyLength -ge 0 -and $bodyLength -ne $span) { return $result }
            # Chunked 206 is legal: Content-Range still bounds decoded bytes.
            $bodyLength = $span
            $result.Total = $total
        } else { return $result }
        if ($result.Total -gt $maximumBytes -or $bodyLength -gt $maximumBytes) { return $result }
        $outputStream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Write, [IO.FileShare]::None)
        if ($offset -eq 0) { $outputStream.SetLength(0) }
        $outputStream.Position = $offset
        $inputStream = $response.GetResponseStream()
        $buffer = New-Object byte[] 65536
        [long]$received = 0
        $result.Resume = $true
        while ($true) {
            $remaining = [int][Math]::Max(0, $BudgetMilliseconds - $clock.ElapsedMilliseconds)
            if ($remaining -lt 1) { return $result }
            $readTask = $inputStream.ReadAsync($buffer, 0, $buffer.Length)
            if (-not $readTask.Wait($remaining)) { return $result }
            $count = $readTask.GetAwaiter().GetResult()
            if ($count -eq 0) { break }
            if ($offset + $received + $count -gt $maximumBytes -or
                ($bodyLength -ge 0 -and $received + $count -gt $bodyLength) -or
                ($result.Total -ge 0 -and $offset + $received + $count -gt $result.Total)) {
                $result.Resume = $false; return $result
            }
            $outputStream.Write($buffer, 0, $count)
            $received += $count
        }
        # A complete response with the wrong hash is a bad source, not a
        # reason to keep retrying its complete bytes. Short bodies may resume.
        if (($bodyLength -lt 0 -or $received -eq $bodyLength) -and
            ($result.Total -lt 0 -or $offset + $received -eq $result.Total)) { $result.Resume = $false }
    } catch {
        # Never expose exception messages: custom source URLs may be private.
        Write-Verbose 'Verified source transfer interrupted; only valid partial bytes may resume.'
    } finally {
        if ($request) { $request.Abort() }
        if ($inputStream) { $inputStream.Dispose() }
        if ($outputStream) { $outputStream.Dispose() }
        if ($response) { $response.Dispose() }
        [Net.ServicePointManager]::SecurityProtocol = $previousTls
    }
    return $result
}

function Invoke-Vpn233VerifiedDownload {
    param([string]$Url, [string]$Destination, [string]$ExpectedSha256)
    $seconds = 300
    if ($env:VPN233_GH_MAX_TIME) {
        $seconds = 0
        if (-not [int]::TryParse($env:VPN233_GH_MAX_TIME, [ref]$seconds) -or $seconds -lt 1) { throw 'VPN233_GH_MAX_TIME must be a positive number of seconds.' }
    }
    $temporary = $Destination + '.' + [guid]::NewGuid().ToString('N') + '.part'
    $clock = [Diagnostics.Stopwatch]::StartNew()
    [long]$total = -1
    try {
        $created = [IO.File]::Open($temporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        $created.Dispose()
        if ($env:OS -ne 'Windows_NT') {
            & chmod 600 -- $temporary
            if ($LASTEXITCODE -ne 0) { return $false }
        }
        for ($attempt = 0; $attempt -lt 3; $attempt++) {
            $remaining = [Math]::Min([int]::MaxValue, ([long]$seconds * 1000) - $clock.ElapsedMilliseconds)
            if ($remaining -lt 1) { break }
            $transfer = Invoke-Vpn233RangeFetch $Url $temporary $total ([int]$remaining)
            $total = $transfer.Total
            $length = (Get-Item -LiteralPath $temporary).Length
            # SHA proves full content even if the peer resets after the last
            # byte. This is the only successful rename/cache path.
            if ($length -gt 0 -and (Get-FileHash -LiteralPath $temporary -Algorithm SHA256).Hash -ieq $ExpectedSha256) {
                Move-Item -LiteralPath $temporary -Destination $Destination -Force
                return $true
            }
            if (-not $transfer.Resume -or $length -eq 0) { break }
        }
        return $false
    } catch { return $false }
    finally { if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force } }
}

function Receive-Vpn233ReleaseAsset {
    param([string]$Asset, [string]$Destination, [string]$Version = '', [switch]$Quiet, [string]$ExpectedSha256 = '')
    if ($ExpectedSha256 -and $ExpectedSha256 -notmatch '\A[a-fA-F0-9]{64}\z') { throw 'Expected SHA-256 must contain exactly 64 hexadecimal characters.' }
    $canonical = Get-Vpn233ReleaseUrl $Asset $Version
    foreach ($prefix in @(Get-Vpn233OrderedMirrorPrefixes release)) {
        $downloaded = if ($ExpectedSha256) { Invoke-Vpn233VerifiedDownload ($prefix + $canonical) $Destination $ExpectedSha256 } else { Invoke-Vpn233AtomicDownload ($prefix + $canonical) $Destination }
        if ($downloaded) {
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
