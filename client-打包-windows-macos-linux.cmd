@echo off
set CLIENT_DIR=client-vpn-neko233

mkdir release-client 2>nul

echo [Client] Building for Windows (amd64)...
set GOOS=windows
set GOARCH=amd64
cd %CLIENT_DIR%
go build -o ../release-client/vpn233-client-windows.exe main.go

echo [Client] Building for macOS (amd64)...
set GOOS=darwin
set GOARCH=amd64
go build -o ../release-client/vpn233-client-macos main.go

echo [Client] Building for Linux (amd64)...
set GOOS=linux
set GOARCH=amd64
go build -o ../release-client/vpn233-client-linux main.go

cd ..

echo Copying config and rules...
copy %CLIENT_DIR%\config.json release-client\config.json
copy %CLIENT_DIR%\rules.yaml release-client\rules.yaml

echo [Client] Build process finished. Output in release-client/
pause
