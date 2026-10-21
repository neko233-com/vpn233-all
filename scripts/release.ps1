[CmdletBinding()]
param([string]$Version = '', [switch]$Publish, [string]$Remote = 'origin')
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$RootDir = Split-Path -Parent $PSScriptRoot
function Invoke-Checked {
    param([string]$Tool, [string[]]$Arguments = @())
    & $Tool @Arguments | Out-Host
    if ($LASTEXITCODE -ne 0) { throw ($Tool + ' failed with code ' + $LASTEXITCODE) }
}
Push-Location $RootDir
try {
    $xTask = & (Join-Path $PSScriptRoot 'xtask.ps1') -PathOnly
    if (-not $Publish) {
        if (-not $Version) { throw '-Version MAJOR.MINOR.PATCH is required to prepare a release' }
        Invoke-Checked $xTask @('version', '--prepare', $Version)
        exit 0
    }
    Invoke-Checked $xTask @('version')
    $current = (Get-Content version.txt -Raw).Trim()
    if ($Version -and $Version -ne $current) { throw 'Prepare and commit the requested version first' }
    $branch = git branch --show-current
    if ($LASTEXITCODE -ne 0 -or $branch -ne 'main') { throw 'Releases must run on main' }
    $dirty = @(git status --porcelain)
    if ($LASTEXITCODE -ne 0 -or $dirty.Count) { throw 'Review and commit all candidate changes before publishing' }
    Invoke-Checked $xTask @('privacy', '--history')
    Invoke-Checked powershell @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'scripts/verify-all.ps1')
    $headSha = git rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve main commit' }
    $remoteRef = git ls-remote --exit-code $Remote refs/heads/main
    if ($LASTEXITCODE -ne 0 -or ($remoteRef -split '\s+')[0] -ne $headSha) { throw 'Push the verified main commit before publishing a tag' }
    $runsJson = gh run list --workflow ci.yml --branch main --commit $headSha --limit 1 --json status,conclusion
    if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect CI status' }
    $runs = @($runsJson | ConvertFrom-Json)
    if (-not $runs.Count -or $runs[0].status -ne 'completed' -or $runs[0].conclusion -ne 'success') { throw 'The main commit needs a successful completed CI run' }
    $tag = 'v' + $current
    $null = git rev-parse --verify --quiet ('refs/tags/' + $tag)
    if ($LASTEXITCODE -eq 0) { throw 'Release tag already exists locally; refusing to replace it' }
    $remoteTag = @(git ls-remote $Remote ('refs/tags/' + $tag))
    if ($LASTEXITCODE -ne 0 -or $remoteTag.Count) { throw 'Cannot publish an existing or unverified remote tag' }
    Invoke-Checked git @('tag', '-a', $tag, '-m', ('Release ' + $tag))
    Invoke-Checked $xTask @('privacy', '--history')
    Invoke-Checked git @('push', $Remote, ('refs/tags/' + $tag + ':refs/tags/' + $tag))
    Write-Host ('Tag ' + $tag + ' pushed; archives will be verified before publication.')
}
finally { Pop-Location }
