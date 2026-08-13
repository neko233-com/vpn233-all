$ErrorActionPreference = 'Stop'
$rootDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$binDir = if ($env:VPN233_BIN_DIR) { $env:VPN233_BIN_DIR } else { Join-Path $rootDir 'generated' }
$webDir = if ($env:VPN233_WEB_DIR) { $env:VPN233_WEB_DIR } else { Join-Path $rootDir 'admin-web\dist' }
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("vpn233-update-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tempDir | Out-Null
try {
  $bundle = $null
  if ($env:VPN233_UPDATE_BUNDLE -and (Test-Path -LiteralPath $env:VPN233_UPDATE_BUNDLE)) {
    $isZip = $env:VPN233_UPDATE_BUNDLE -match '(?i)\.zip$'
    $bundle = Join-Path $tempDir $(if ($isZip) { 'bundle.zip' } else { 'bundle.tar.gz' })
    Copy-Item -LiteralPath $env:VPN233_UPDATE_BUNDLE -Destination $bundle
  } else {
    $sources = if ($env:VPN233_UPDATE_URLS) { $env:VPN233_UPDATE_URLS -split '\|' } else { @('https://github.com/neko233-com/vpn233-all/releases/latest/download/vpn233-rust-windows-x86_64.zip', 'https://ghproxy.net/https://github.com/neko233-com/vpn233-all/releases/latest/download/vpn233-rust-windows-x86_64.zip', 'https://github.com/neko233-com/vpn233-all/releases/latest/download/vpn233-rust-windows-x86_64.tar.gz') }
    $downloaded = $false
    foreach ($source in $sources) {
      Write-Host "Trying update source: $source"
      try {
        $isZip = $source -match '(?i)\.zip(?:\?|$)'
        $bundle = Join-Path $tempDir $(if ($isZip) { 'bundle.zip' } else { 'bundle.tar.gz' })
        Invoke-WebRequest -UseBasicParsing -Uri $source -OutFile $bundle -TimeoutSec 120
        $downloaded = $true
        break
      } catch { Write-Warning $_.Exception.Message }
    }
    if (-not $downloaded) { throw 'No update source reachable. Use VPN233_UPDATE_BUNDLE for an offline bundle.' }
  }
  if ($env:VPN233_UPDATE_SHA256) { if ((Get-FileHash -Algorithm SHA256 -LiteralPath $bundle).Hash -ne $env:VPN233_UPDATE_SHA256.ToUpperInvariant()) { throw 'SHA-256 verification failed' } }
  if (-not $env:VPN233_UPDATE_SHA256 -and -not $env:VPN233_ALLOW_UNSIGNED_UPDATE) { throw 'Refusing remote update without VPN233_UPDATE_SHA256. Set the published SHA-256 or explicitly opt into an unsigned update with VPN233_ALLOW_UNSIGNED_UPDATE=1.' }
  $extract = Join-Path $tempDir 'extract'
  New-Item -ItemType Directory -Force -Path $extract | Out-Null
  if ($bundle -match '(?i)\.zip$') {
    Expand-Archive -LiteralPath $bundle -DestinationPath $extract -Force
  } else {
    if (-not (Get-Command tar.exe -ErrorAction SilentlyContinue)) { throw 'tar.exe is required to extract .tar.gz update bundles.' }
    & tar.exe -xzf $bundle -C $extract
    if ($LASTEXITCODE -ne 0) { throw 'Failed to extract the .tar.gz update bundle.' }
  }
  New-Item -ItemType Directory -Force -Path $binDir | Out-Null
  Get-ChildItem -LiteralPath $extract -Recurse -File -Filter 'vpn233-*.exe' | Copy-Item -Destination $binDir -Force
  $webSource = Get-ChildItem -LiteralPath $extract -Recurse -Directory | Where-Object { $_.FullName -match '(?i)[\\/]admin-web[\\/]dist$' } | Select-Object -First 1
  if (-not $webSource) {
    $webIndex = Get-ChildItem -LiteralPath $extract -Recurse -File -Filter 'index.html' | Where-Object { $_.FullName -match '(?i)[\\/]admin-web[\\/]' } | Select-Object -First 1
    if ($webIndex) { $webSource = $webIndex.Directory }
  }
  if ($webSource) {
    New-Item -ItemType Directory -Force -Path $webDir | Out-Null
    Get-ChildItem -LiteralPath $webSource.FullName -Force | Copy-Item -Destination $webDir -Recurse -Force
  }
  Write-Host "Update installed into $binDir"
} finally { Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue }
