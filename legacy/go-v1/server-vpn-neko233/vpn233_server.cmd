@echo off
setlocal enabledelayedexpansion

set BINARY=vpn233-server.exe
set CONFIG=config.json
set LOG=vpn233-server.log

if "%1"=="" goto usage
if "%1"=="start" goto start
if "%1"=="stop" goto stop
if "%1"=="status" goto status
if "%1"=="restart" goto restart

:usage
echo Usage: vpn233_server.cmd {start^|stop^|status^|restart}
exit /b 1

:start
tasklist /FI "IMAGENAME eq %BINARY%" 2>NUL | find /I /N "%BINARY%" >NUL
if "%ERRORLEVEL%"=="0" (
    echo [INFO] %BINARY% is already running.
    exit /b 0
)
echo [INFO] Starting %BINARY%...
start /B %BINARY% -c %CONFIG% > %LOG% 2>&1
timeout /t 2 >nul
tasklist /FI "IMAGENAME eq %BINARY%" 2>NUL | find /I /N "%BINARY%" >NUL
if "%ERRORLEVEL%"=="0" (
    echo [SUCCESS] %BINARY% started successfully.
) else (
    echo [ERROR] Failed to start %BINARY%. Check %LOG% for details.
)
exit /b 0

:stop
tasklist /FI "IMAGENAME eq %BINARY%" 2>NUL | find /I /N "%BINARY%" >NUL
if "%ERRORLEVEL%" neq "0" (
    echo [INFO] %BINARY% is not running.
    exit /b 0
)
echo [INFO] Stopping %BINARY%...
taskkill /F /IM %BINARY% /T >nul 2>&1
echo [SUCCESS] %BINARY% stopped.
exit /b 0

:status
tasklist /FI "IMAGENAME eq %BINARY%" 2>NUL | find /I /N "%BINARY%" >NUL
if "%ERRORLEVEL%"=="0" (
    for /f "tokens=2" %%a in ('tasklist /FI "IMAGENAME eq %BINARY%" /NH') do set pid=%%a
    echo [STATUS] %BINARY% is RUNNING (PID: !pid!)
) else (
    echo [STATUS] %BINARY% is STOPPED
)
exit /b 0

:restart
call :stop
call :start
exit /b 0
