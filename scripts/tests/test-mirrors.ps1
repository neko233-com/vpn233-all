# Entirely offline: the network primitive is replaced by an in-process fake.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../lib/mirrors.ps1')
$work = Join-Path ([IO.Path]::GetTempPath()) ('vpn233-mirrors-test-' + [guid]::NewGuid().ToString('N'))
$variables = @('VPN233_HOME', 'VPN233_GH_MIRROR', 'VPN233_GH_MIRRORS', 'VPN233_NO_MIRROR', 'VPN233_MIRROR_MODE')
$saved = @{}
foreach ($variable in $variables) { $saved[$variable] = @{ Present = (Test-Path -LiteralPath ('Env:' + $variable)); Value = [Environment]::GetEnvironmentVariable($variable, 'Process') } }
New-Item -ItemType Directory -Path $work | Out-Null
$script:passed = 0
function Assert-Equal {
    param([string]$Name, $Expected, $Actual)
    if ($Expected -cne $Actual) { throw ('FAIL: ' + $Name) }
    $script:passed++; Write-Host ('PASS: ' + $Name)
}
function Assert-Rejected {
    param([string]$Name, [scriptblock]$Action)
    $rejected = $false
    try { & $Action | Out-Null } catch { $rejected = $true }
    Assert-Equal $Name $true $rejected
}
try {
    foreach ($variable in $variables) { if (Test-Path -LiteralPath ('Env:' + $variable)) { Remove-Item -LiteralPath ('Env:' + $variable) } }
    $env:VPN233_HOME = Join-Path $work 'home'
    Assert-Equal 'contract and direct fallback' 'https://ghproxy.net/|https://gh.llkk.cc/|https://ghfast.top/|https://gh-proxy.com/|' ((Get-Vpn233MirrorPrefixes) -join '|')
    Assert-Equal 'latest URL' 'https://github.com/neko233-com/vpn233-all/releases/latest/download/SHA256SUMS' (Get-Vpn233ReleaseUrl SHA256SUMS)
    Assert-Equal 'pinned URL' 'https://github.com/neko233-com/vpn233-all/releases/download/v2.1.0/SHA256SUMS' (Get-Vpn233ReleaseUrl SHA256SUMS v2.1.0)
    Assert-Rejected 'version path traversal rejected' { Get-Vpn233Version '../main' }
    Assert-Rejected 'invalid expected archive hash rejected before download' { Receive-Vpn233ReleaseAsset fixture.bin (Join-Path $work 'must-not-exist.bin') -ExpectedSha256 'invalid' }
    Assert-Rejected 'expected archive hash with trailing LF rejected before download' { Receive-Vpn233ReleaseAsset fixture.bin (Join-Path $work 'must-not-exist.bin') -ExpectedSha256 (('a' * 64) + "`n") }
    Assert-Equal 'invalid expected archive hash does not create output' $false (Test-Path -LiteralPath (Join-Path $work 'must-not-exist.bin'))
    foreach ($prefix in @('example.invalid', 'https://user:private@example.invalid/', 'https://example.invalid/?private=secret', 'https://example.invalid/#fragment', 'https://example.invalid/ has-space')) {
        Assert-Rejected 'unsafe mirror rejected without logging value' { ConvertTo-Vpn233MirrorPrefix $prefix }
    }
    $env:VPN233_GH_MIRRORS = 'https://one.invalid/||https://two.invalid/'
    Assert-Equal 'configured direct fallback retained' 'https://one.invalid/||https://two.invalid/' ((Get-Vpn233MirrorPrefixes) -join '|')
    Set-Vpn233MirrorMode off
    Assert-Equal 'off overrides environment list' '' ((Get-Vpn233MirrorPrefixes) -join '|')
    Set-Vpn233MirrorMode auto
    $script:calls = @()
    $script:failPattern = 'one.invalid'
    $script:empty = $false
    function Invoke-Vpn233Fetch {
        param([string]$Url, [string]$Destination)
        $script:calls += $Url
        if ($script:failPattern -and $Url.Contains($script:failPattern)) { throw 'Synthetic network failure' }
        [IO.File]::WriteAllText($Destination, $(if ($script:empty) { '' } else { 'fixture bytes' }))
    }
    $destination = Join-Path $work 'asset.bin'
    Receive-Vpn233ReleaseAsset SHA256SUMS $destination -Quiet -WarningAction SilentlyContinue
    Assert-Equal 'fallback stops after success' 2 $script:calls.Count
    Assert-Equal 'fallback content' 'fixture bytes' ([IO.File]::ReadAllText($destination))
    Assert-Equal 'selected direct source is cached' 'release ' ([IO.File]::ReadAllLines((Get-Vpn233MirrorCachePath))[0])
    $script:calls = @()
    Receive-Vpn233ReleaseAsset SHA256SUMS $destination -Quiet -WarningAction SilentlyContinue
    Assert-Equal 'cached direct first' 'https://github.com/neko233-com/vpn233-all/releases/latest/download/SHA256SUMS' $script:calls[0]
    $script:failPattern = 'https://'
    Assert-Rejected 'all sources fail' { Receive-Vpn233ReleaseAsset SHA256SUMS $destination -Quiet -WarningAction SilentlyContinue }
    Assert-Equal 'failure preserves prior destination' 'fixture bytes' ([IO.File]::ReadAllText($destination))
    $script:failPattern = ''; $script:empty = $true
    Assert-Rejected 'empty download rejected' { Receive-Vpn233ReleaseAsset SHA256SUMS $destination -Quiet -WarningAction SilentlyContinue }
    Assert-Equal 'empty download preserves destination' 'fixture bytes' ([IO.File]::ReadAllText($destination))
    $sum = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash
    Test-Vpn233Checksum $destination $sum
    Assert-Rejected 'mismatched checksum rejected' { Test-Vpn233Checksum $destination ('0' * 64) }
    Assert-Rejected 'invalid checksum rejected' { Test-Vpn233Checksum $destination ($sum + 'junk') }
    $sums = Join-Path $work 'SHA256SUMS'
    [IO.File]::WriteAllText($sums, "$sum *asset.bin`r`n")
    Assert-Equal 'star and CRLF checksum parsing' $sum.ToLowerInvariant() (Get-Vpn233Checksum $sums asset.bin)
    Assert-Rejected 'missing checksum entry rejected' { Get-Vpn233Checksum $sums absent.bin }
    [IO.File]::AppendAllText($sums, "$sum  asset.bin`n")
    Assert-Rejected 'duplicate checksum entry rejected' { Get-Vpn233Checksum $sums asset.bin }
    Assert-Equal 'no failed download parts left' 0 @(Get-ChildItem -LiteralPath $work -Filter '*.part').Count
    Write-Host ('test-mirrors.ps1: ' + $script:passed + ' passed')
} finally {
    foreach ($variable in $variables) {
        if ($saved[$variable].Present) { Set-Item -LiteralPath ('Env:' + $variable) -Value $saved[$variable].Value }
        elseif (Test-Path -LiteralPath ('Env:' + $variable)) { Remove-Item -LiteralPath ('Env:' + $variable) }
    }
    Remove-Item -LiteralPath $work -Recurse -Force
}
