@echo off
echo [Server] Building for Linux (amd64)...
set GOOS=linux
set GOARCH=amd64

mkdir release-server 2>nul

cd server-vpn-neko233
go build -o ../release-server/vpn233-server main.go
cd ..

if %ERRORLEVEL% EQU 0 (
    echo [Server] Build successful: release-server/vpn233-server
    copy server-vpn-neko233\config.json release-server\config.json
) else (
    echo [Server] Build failed!
)
pause
