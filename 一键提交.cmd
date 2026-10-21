@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\commit.ps1" %*
exit /b %ERRORLEVEL%
