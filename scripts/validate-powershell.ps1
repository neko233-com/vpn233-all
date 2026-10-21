[CmdletBinding()]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$RootDir = Split-Path -Parent $PSScriptRoot
Push-Location $RootDir
try {
    Import-Module PSScriptAnalyzer -ErrorAction Stop
    $paths = @(git ls-files --cached --others --exclude-standard)
    if ($LASTEXITCODE -ne 0) { throw 'Cannot enumerate Git candidates' }
    $files = @($paths | Sort-Object -Unique | Where-Object { $_ -notlike 'legacy/*' -and $_.EndsWith('.ps1') -and (Test-Path -LiteralPath $_) })
    foreach ($file in $files) {
        $tokens = $null
        $parseErrors = $null
        $null = [Management.Automation.Language.Parser]::ParseFile((Join-Path $RootDir $file), [ref]$tokens, [ref]$parseErrors)
        if ($parseErrors.Count) { throw ('PowerShell parse failed: ' + $file) }
        $findings = @(Invoke-ScriptAnalyzer -Path $file -Settings '.github/psscriptanalyzer/PSScriptAnalyzerSettings.psd1')
        if ($findings.Count) { $findings | Format-Table -AutoSize | Out-Host; throw ('PSScriptAnalyzer failed: ' + $file) }
    }
    Write-Host ('PowerShell: ' + $files.Count + ' scripts passed parse and analyzer')
}
finally { Pop-Location }
