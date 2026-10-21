# Forward the documented install.ps1 parameters without evaluating strings.
& (Join-Path $PSScriptRoot 'install.ps1') -Release @args
if ($LASTEXITCODE) { exit $LASTEXITCODE }
