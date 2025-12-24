@echo off
echo Stopping VPN233 System...

taskkill /F /IM main.exe /T 2>nul
taskkill /F /FI "WINDOWTITLE eq VPN233-Server" /T 2>nul
taskkill /F /FI "WINDOWTITLE eq VPN233-Client" /T 2>nul

echo VPN233 stopped.
pause
