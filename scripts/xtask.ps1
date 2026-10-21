# Build the native Rust development tool independently of product flags.
param([switch]$PathOnly)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ToolArguments = @($args)
$workspaceRoot = Split-Path -Parent $PSScriptRoot
$bootstrapTarget = Join-Path $workspaceRoot 'target/xtask-bootstrap'
$savedEnvironment = @{}
foreach ($name in @('CARGO_ENCODED_RUSTFLAGS', 'RUSTFLAGS', 'CARGO_BUILD_TARGET')) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
    [Environment]::SetEnvironmentVariable($name, $null, 'Process')
}
Push-Location $workspaceRoot
try {
    & cargo build --locked --quiet -p vpn233-xtask --target-dir $bootstrapTarget
    if ($LASTEXITCODE -ne 0) { throw 'Cannot build the Rust development tool.' }
} finally {
    Pop-Location
    foreach ($name in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process') }
}
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
