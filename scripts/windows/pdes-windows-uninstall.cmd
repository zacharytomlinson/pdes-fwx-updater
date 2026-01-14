@echo off
setlocal

set "SCRIPT=%~dp0pdes-windows-uninstall.ps1"

if not exist "%SCRIPT%" (
    echo PDES uninstall script not found at "%SCRIPT%".
    exit /b 1
)

rem Elevate and run the PowerShell uninstaller so Program Files can be removed.
powershell -NoProfile -ExecutionPolicy Bypass -Command ^
  "Start-Process PowerShell -Verb RunAs -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','\"%SCRIPT%\"'"  || exit /b 1

exit /b 0
