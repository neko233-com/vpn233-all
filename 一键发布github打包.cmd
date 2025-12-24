@echo off
setlocal enabledelayedexpansion

:: 调用 PowerShell 脚本执行核心逻辑
powershell -ExecutionPolicy Bypass -File "release.ps1"

exit /b

pause