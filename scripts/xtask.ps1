# Build the native Rust development tool independently of product flags.
[Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidGlobalVars', '', Justification = 'Preserve the native Cargo exit code for callers of -PathOnly.')]
param([switch]$PathOnly)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ToolArguments = @($args)
$workspaceRoot = Split-Path -Parent $PSScriptRoot
# Product target cache pruning must never remove this independent tool.
$bootstrapTarget = Join-Path $workspaceRoot '.cache/xtask-bootstrap'
$cargo = Get-Command cargo -CommandType Application -ErrorAction Stop | Select-Object -First 1
$buildInfo = New-Object Diagnostics.ProcessStartInfo
$buildInfo.FileName = $cargo.Path
$buildInfo.WorkingDirectory = $workspaceRoot
$buildInfo.UseShellExecute = $false
foreach ($name in @('CARGO_ENCODED_RUSTFLAGS', 'RUSTFLAGS', 'CARGO_BUILD_TARGET')) { $buildInfo.EnvironmentVariables.Remove($name) }
$buildArguments = @('build', '--locked', '--quiet', '-p', 'vpn233-xtask', '--target-dir', $bootstrapTarget)
if ($buildInfo.PSObject.Properties.Name -contains 'ArgumentList') {
    foreach ($argument in $buildArguments) { $buildInfo.ArgumentList.Add($argument) }
} else {
    # Windows PowerShell 5.1 has no ArgumentList. Quote every argument using
    # the Windows native rules, including quotes and trailing backslashes.
    $quotedArguments = foreach ($argument in $buildArguments) {
        $escaped = [regex]::Replace($argument, '(\\*)"', '$1$1\"')
        $escaped = [regex]::Replace($escaped, '(\\+)$', '$1$1')
        '"' + $escaped + '"'
    }
    $buildInfo.Arguments = $quotedArguments -join ' '
}
$build = New-Object Diagnostics.Process
try {
    $build.StartInfo = $buildInfo
    if (-not $build.Start()) { throw 'Cannot start the Rust development tool build.' }
    $build.WaitForExit()
    $global:LASTEXITCODE = $build.ExitCode
    if ($build.ExitCode -ne 0) { throw 'Cannot build the Rust development tool.' }
} finally { $build.Dispose() }
$suffix = if ($env:OS -eq 'Windows_NT') { '.exe' } else { '' }
$tool = Join-Path $bootstrapTarget ('debug/vpn233-xtask' + $suffix)
if (-not (Test-Path -LiteralPath $tool -PathType Leaf)) { throw 'Rust development tool is missing.' }
if ($PathOnly) {
    if ($ToolArguments.Count) { throw '-PathOnly takes no additional arguments.' }
    if ($env:OS -eq 'Windows_NT') { Write-Output $tool.Replace('\', '/') } else { Write-Output $tool }
    return
}
Push-Location $workspaceRoot
try { & $tool @ToolArguments; $toolStatus = $LASTEXITCODE }
finally { Pop-Location }
exit $toolStatus
