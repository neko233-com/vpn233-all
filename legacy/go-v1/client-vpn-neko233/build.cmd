@echo off
echo Building for Windows (amd64)...
set GOOS=windows
set GOARCH=amd64
go build -o client-vpn-neko233-win.exe main.go

echo Building for macOS (amd64)...
set GOOS=darwin
set GOARCH=amd64
go build -o client-vpn-neko233-macos main.go

if %ERRORLEVEL% EQU 0 (
    echo Build successful.
) else (
    echo Build failed!
)
pause
