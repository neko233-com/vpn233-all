@echo off
setlocal
set "ROOT_DIR=%~dp0"
set "BIN_DIR=%VPN233_BIN_DIR%"
if "%BIN_DIR%"=="" set "BIN_DIR=%ROOT_DIR%generated"
if not exist "%BIN_DIR%\vpn233-provider-server.exe" if exist "%ROOT_DIR%target\release\vpn233-provider-server.exe" set "BIN_DIR=%ROOT_DIR%target\release"
set "CONFIG_DIR=%VPN233_CONFIG_DIR%"
if "%CONFIG_DIR%"=="" set "CONFIG_DIR=%ROOT_DIR%config"
if not exist "%BIN_DIR%\vpn233-provider-server.exe" (
  echo Missing Rust binaries. Run install.ps1 first.
  exit /b 1
)
if not exist "%CONFIG_DIR%" mkdir "%CONFIG_DIR%"
start "VPN233 Provider" /B "%BIN_DIR%\vpn233-provider-server.exe" --config "%CONFIG_DIR%\provider-server.yaml"
start "VPN233 Subscribe" /B "%BIN_DIR%\vpn233-subscribe-server.exe" --config "%CONFIG_DIR%\subscribe-server.yaml"
start "VPN233 Client" /B "%BIN_DIR%\vpn233-client.exe" --config "%CONFIG_DIR%\vpn233-client.yaml"
echo VPN233 Rust services started.
