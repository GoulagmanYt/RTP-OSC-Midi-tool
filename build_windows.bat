@echo off
setlocal EnableExtensions DisableDelayedExpansion
title OSCMidi - Build Assistant
cd /d "%~dp0"
chcp 65001 >nul 2>nul

set "NO_COLOR="
set "FORCE_COLOR=1"
set "CARGO_TERM_COLOR=always"
rem Let the selected PowerShell edition rebuild its own module search path.
set "PSModulePath="
set "BUILD_SCRIPT=%~dp0scripts\build-windows.ps1"
set "POWERSHELL_EXE=pwsh.exe"
where pwsh.exe >nul 2>nul || set "POWERSHELL_EXE=powershell.exe"

if not exist "%BUILD_SCRIPT%" (
  echo [ERROR] Build script not found: "%BUILD_SCRIPT%"
  endlocal & exit /b 2
)

rem Double-click opens the assistant. Explicit options remain automation-friendly.
if "%~1"=="" (
  "%POWERSHELL_EXE%" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%BUILD_SCRIPT%" -Interactive
) else (
  "%POWERSHELL_EXE%" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%BUILD_SCRIPT%" %*
)
set "BUILD_EXIT_CODE=%ERRORLEVEL%"
endlocal & exit /b %BUILD_EXIT_CODE%
