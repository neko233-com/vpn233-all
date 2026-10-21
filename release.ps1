# Compatibility entry point; preparation is local unless -Publish is explicit.
[CmdletBinding()]
param([string]$Version = '', [switch]$Publish, [string]$Remote = 'origin')
& (Join-Path $PSScriptRoot 'scripts/release.ps1') -Version $Version -Publish:$Publish -Remote $Remote
exit $LASTEXITCODE
