$ErrorActionPreference = 'Stop'
$rootDir = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
& (Join-Path $rootDir 'install.ps1') @args

