$ErrorActionPreference = 'Stop'
$rootDir = Split-Path -Parent $MyInvocation.MyCommand.Path
if ($args.Count -gt 0 -and $args[0] -eq 'update') { & (Join-Path $rootDir 'update.ps1'); exit $LASTEXITCODE }
$destBinDir = if ($env:VPN233_BIN_DIR) { $env:VPN233_BIN_DIR } else { Join-Path $rootDir 'generated' }
$webDestDir = if ($env:VPN233_WEB_DIR) { $env:VPN233_WEB_DIR } else { Join-Path $rootDir 'admin-web\dist' }
$configDestDir = if ($env:VPN233_CONFIG_DIR) { $env:VPN233_CONFIG_DIR } else { Join-Path $rootDir 'config' }
$binSourceDir = $env:VPN233_SOURCE_BIN_DIR
if (-not $binSourceDir) {
  foreach ($candidate in @((Join-Path $rootDir 'bin'), (Join-Path $rootDir 'generated'), (Join-Path $rootDir 'target\release'))) {
    if ((Test-Path (Join-Path $candidate 'vpn233-provider-server.exe')) -and
        (Test-Path (Join-Path $candidate 'vpn233-subscribe-server.exe')) -and
        (Test-Path (Join-Path $candidate 'vpn233-client.exe'))) {
      $binSourceDir = $candidate
      break
    }
  }
}

if (-not $binSourceDir) {
  if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { throw 'No prebuilt Rust binaries found and Cargo is unavailable. Install Rust from https://rustup.rs or set VPN233_SOURCE_BIN_DIR.' }
  Push-Location $rootDir
  try { cargo build --release --workspace } finally { Pop-Location }
  $binSourceDir = Join-Path $rootDir 'target\release'
}

$webSourceDir = Join-Path $rootDir 'admin-web\dist'
if (-not (Test-Path (Join-Path $webSourceDir 'index.html'))) {
  $flatWebDir = Join-Path $rootDir 'admin-web'
  if (Test-Path (Join-Path $flatWebDir 'index.html')) {
    $webSourceDir = $flatWebDir
  } elseif (Get-Command npm -ErrorAction SilentlyContinue) {
    Push-Location (Join-Path $rootDir 'admin-web')
    try { npm.cmd ci; npm.cmd run build } finally { Pop-Location }
  } else {
    throw 'No admin web build found and Node.js/npm is unavailable. Provide admin-web/dist or install Node.js.'
  }
}

New-Item -ItemType Directory -Force -Path $destBinDir | Out-Null
foreach ($binary in @('vpn233-provider-server', 'vpn233-subscribe-server', 'vpn233-client')) {
  Copy-Item -LiteralPath (Join-Path $binSourceDir "$binary.exe") -Destination (Join-Path $destBinDir "$binary.exe") -Force
}
if ([System.IO.Path]::GetFullPath($webSourceDir) -ne [System.IO.Path]::GetFullPath($webDestDir)) {
  New-Item -ItemType Directory -Force -Path $webDestDir | Out-Null
  Get-ChildItem -LiteralPath $webSourceDir -Force | Copy-Item -Destination $webDestDir -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $configDestDir | Out-Null
foreach ($config in @('provider-server', 'subscribe-server', 'vpn233-client')) {
  $destination = Join-Path $configDestDir "$config.yaml"
  if (-not (Test-Path -LiteralPath $destination)) {
    $source = Join-Path $rootDir "config\$config.yaml"
    if (-not (Test-Path -LiteralPath $source)) { $source = Join-Path $rootDir "$config\config.example.yaml" }
    if (Test-Path -LiteralPath $source) { Copy-Item -LiteralPath $source -Destination $destination -Force }
  }
}
Write-Host "Installed Rust services in $destBinDir"
Write-Host "Installed admin web in $webDestDir"
Write-Host "Configuration templates are available in $configDestDir; existing files were preserved."
