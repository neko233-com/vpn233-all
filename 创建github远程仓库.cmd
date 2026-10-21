@echo off
rem Compatibility diagnostic. Public visibility also exposes historical refs.
gh repo view neko233-com/vpn233-all --json nameWithOwner,visibility,url
if errorlevel 1 exit /b %ERRORLEVEL%
echo Run the full privacy and history checks before changing repository visibility.
exit /b 0
