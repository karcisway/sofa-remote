@echo off
rem Sofa Remote developer commands. Run "cli help" for the list; the work is done by cli.ps1.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0cli.ps1" %*
exit /b %ERRORLEVEL%
