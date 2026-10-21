@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\verify-all.ps1" %*
exit /b %ERRORLEVEL%
