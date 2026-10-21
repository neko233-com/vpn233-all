# Commit an explicitly staged change after local checks. Never stages files.
[CmdletBinding()]
param([string]$Message = 'chore: update VPN233', [switch]$Push)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
function Invoke-Checked {
    param([string]$Tool, [string[]]$Arguments)
    & $Tool @Arguments | Out-Host
    if ($LASTEXITCODE -ne 0) { throw ($Tool + ' failed with code ' + $LASTEXITCODE) }
}
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    if ((git branch --show-current) -ne 'main' -or $LASTEXITCODE -ne 0) { throw 'Only main is supported.' }
    git diff --cached --quiet
    if ($LASTEXITCODE -eq 0) { throw 'Stage the intended public files explicitly before committing.' }
    if ($LASTEXITCODE -ne 1) { throw 'Cannot inspect the index.' }
    if ($Push) {
        git diff --quiet
        if ($LASTEXITCODE -ne 0) { throw 'Push verification requires no unstaged tracked changes.' }
        $untracked = @(git ls-files --others --exclude-standard)
        if ($LASTEXITCODE -ne 0 -or $untracked.Count) { throw 'Stage or ignore remaining candidate files before push verification.' }
    }
    $stagedTree = git write-tree
    if ($LASTEXITCODE -ne 0) { throw 'Cannot snapshot the staged tree.' }
    $xTask = & (Join-Path $PSScriptRoot 'xtask.ps1') -PathOnly
    Invoke-Checked $xTask @('privacy')
    Invoke-Checked git @('diff', '--cached', '--check')
    $parameters = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'scripts/verify-all.ps1')
    if (-not $Push) { $parameters += '-Quick' }
    Invoke-Checked powershell $parameters
    if ((git write-tree) -ne $stagedTree -or $LASTEXITCODE -ne 0) { throw 'The index changed during verification; run again.' }
    if ($Push) {
        git diff --quiet
        if ($LASTEXITCODE -ne 0) { throw 'Source changed during verification; run again.' }
    }
    Invoke-Checked git @('commit', '-m', $Message)
    if ($Push) { Invoke-Checked git @('push', 'origin', 'main') }
}
finally { Pop-Location }
