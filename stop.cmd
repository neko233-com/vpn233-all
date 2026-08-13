@echo off
taskkill /IM vpn233-provider-server.exe /T /F 2>nul
taskkill /IM vpn233-subscribe-server.exe /T /F 2>nul
taskkill /IM vpn233-client.exe /T /F 2>nul
echo VPN233 Rust services stopped.
