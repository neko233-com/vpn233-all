@echo off
echo Building for Linux (amd64)...
set GOOS=linux
set GOARCH=amd64
go build -o server-vpn-neko233 main.go
if %ERRORLEVEL% EQU 0 (
    echo Build successful: server-vpn-neko233
) else (
    echo Build failed!
)
pause
