@echo off
echo Starting VPN233 System...

:: Start Server
start /B "VPN233-Server" go run server-vpn-neko233/main.go -c server-vpn-neko233/config.json

:: Start Client
start /B "VPN233-Client" go run client-vpn-neko233/main.go -c client-vpn-neko233/config.json

echo VPN233 is running in background.
echo Client Web: http://127.0.0.1:8082
echo Server Web: http://127.0.0.1:8083
pause
